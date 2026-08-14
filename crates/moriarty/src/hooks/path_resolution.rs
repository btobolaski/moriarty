//! Shared path resolution for hook authorization checks.
//!
//! Tool locality and Bash redirect policy both need the same fail-closed handling for existing
//! symlinks and not-yet-created targets. Keeping the primitive here prevents the two policy layers
//! from drifting on parent traversal, broken symlinks, or missing suffixes.

use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

fn is_missing_path_error(error: &io::Error, allow_non_directory: bool) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || allow_non_directory
            && (error.kind() == io::ErrorKind::NotADirectory
                // Older Windows/Rust combinations report ERROR_DIRECTORY as Other.
                || cfg!(windows) && is_windows_error_directory(error))
}

fn is_windows_error_directory(error: &io::Error) -> bool {
    error.raw_os_error() == Some(267)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RedirectTargetResolution {
    pub(crate) match_text: String,
    pub(crate) resolved_path: PathBuf,
    pub(crate) is_local: bool,
    pub(crate) is_device_or_special: bool,
}

#[derive(Debug)]
pub(crate) struct RedirectResolutionContext {
    cwd: Option<PathBuf>,
    canonical_cwd: OnceLock<Result<PathBuf, String>>,
    home: Option<PathBuf>,
    canonical_home: OnceLock<Result<PathBuf, String>>,
}

impl RedirectResolutionContext {
    pub(crate) fn new(cwd: &Path, home: Option<&Path>) -> Self {
        Self {
            cwd: (!cwd.as_os_str().is_empty()).then(|| cwd.to_path_buf()),
            canonical_cwd: OnceLock::new(),
            home: home.map(Path::to_path_buf),
            canonical_home: OnceLock::new(),
        }
    }

    pub(crate) fn resolve(
        &self,
        target: &str,
        expand_home_tilde: bool,
    ) -> io::Result<RedirectTargetResolution> {
        let candidate = if expand_home_tilde && target == "~" {
            self.home_for_expansion()?.to_path_buf()
        } else if expand_home_tilde && let Some(relative) = target.strip_prefix("~/") {
            self.home_for_expansion()?
                .join(relative.trim_start_matches('/'))
        } else {
            let path = PathBuf::from(target);
            if path.is_absolute() {
                path
            } else {
                self.cwd_for_relative_target()?.join(path)
            }
        };
        let resolved_path = canonicalize_redirect_target(&candidate)?;
        let is_local = self
            .canonical_cwd()
            .ok()
            .is_some_and(|cwd| cwd.parent().is_some() && resolved_path.starts_with(cwd));
        let is_device_or_special = match fs::metadata(&resolved_path) {
            Ok(metadata) => !metadata.is_file() && !metadata.is_dir(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let match_text = self.render_match_text(&resolved_path, is_device_or_special)?;

        Ok(RedirectTargetResolution {
            match_text,
            resolved_path,
            is_local,
            is_device_or_special,
        })
    }

    fn cwd_for_relative_target(&self) -> io::Result<&Path> {
        self.canonical_cwd().map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("redirect target needs an available working directory: {error}"),
            )
        })
    }

    fn canonical_cwd(&self) -> io::Result<&Path> {
        let Some(cwd) = &self.cwd else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no working directory was recorded",
            ));
        };
        match self
            .canonical_cwd
            .get_or_init(|| fs::canonicalize(cwd).map_err(|error| error.to_string()))
        {
            Ok(cwd) => Ok(cwd),
            Err(error) => Err(io::Error::new(io::ErrorKind::NotFound, error.clone())),
        }
    }

    fn home_for_expansion(&self) -> io::Result<&Path> {
        let Some(home) = &self.home else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "HOME is unavailable for redirect target expansion",
            ));
        };
        match self
            .canonical_home
            .get_or_init(|| fs::canonicalize(home).map_err(|error| error.to_string()))
        {
            Ok(home) => Ok(home),
            Err(error) => Err(io::Error::new(io::ErrorKind::NotFound, error.clone())),
        }
    }

    fn render_match_text(&self, path: &Path, is_device_or_special: bool) -> io::Result<String> {
        if is_device_or_special {
            return render_path(path).map(str::to_string);
        }
        if let Ok(cwd) = self.canonical_cwd()
            && let Ok(relative) = path.strip_prefix(cwd)
        {
            return render_cwd_relative(relative);
        }
        if let Ok(home) = self.home_for_expansion()
            && let Ok(relative) = path.strip_prefix(home)
        {
            return render_home_relative(relative);
        }
        render_path(path).map(str::to_string)
    }
}

fn reject_unsafe_virtual_path(path: &Path) -> io::Result<()> {
    if is_unsafe_virtual_path(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "virtual redirect paths cannot be resolved safely by the hook",
        ));
    }
    Ok(())
}

// Linux caps path resolution at 40 links.
const MAX_SYMLINK_HOPS: usize = 40;

fn missing_component(component: Component<'_>) -> io::Result<MissingPathComponent> {
    match component {
        Component::CurDir => Ok(MissingPathComponent::CurDir),
        Component::ParentDir => Ok(MissingPathComponent::ParentDir),
        Component::Normal(name) => Ok(MissingPathComponent::Normal(name.to_os_string())),
        Component::Prefix(_) | Component::RootDir => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "absolute component inside missing path suffix",
        )),
    }
}

fn rebuild_checked_missing_suffix(
    base: PathBuf,
    suffix: Vec<MissingPathComponent>,
    reject_virtual: bool,
) -> io::Result<PathBuf> {
    let rebuilt = rebuild_missing_suffix(base, suffix)?;
    if reject_virtual {
        reject_unsafe_virtual_path(&rebuilt)?;
    }
    Ok(rebuilt)
}

fn resolve_missing_path(
    path: &Path,
    hops: &mut usize,
    allow_missing: bool,
    allow_non_directory: bool,
    reject_virtual: bool,
) -> io::Result<PathBuf> {
    let mut resolved = PathBuf::new();
    let mut components = path.components().peekable();

    while let Some(component) = components.next() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                let candidate = resolved.join(name);
                if reject_virtual {
                    reject_unsafe_virtual_path(&candidate)?;
                }
                let metadata = match fs::symlink_metadata(&candidate) {
                    Ok(metadata) => metadata,
                    Err(error)
                        if allow_missing && is_missing_path_error(&error, allow_non_directory) =>
                    {
                        let mut suffix = vec![MissingPathComponent::Normal(name.to_os_string())];
                        for component in components {
                            suffix.push(missing_component(component)?);
                        }
                        return rebuild_checked_missing_suffix(resolved, suffix, reject_virtual);
                    }
                    Err(error) => return Err(error),
                };

                let resolved_is_dir = if metadata.file_type().is_symlink() {
                    if *hops == MAX_SYMLINK_HOPS {
                        return Err(io::Error::other(
                            "too many symbolic links while checking redirect target",
                        ));
                    }
                    *hops += 1;
                    let target = fs::read_link(&candidate)?;
                    let target = if target.is_absolute() {
                        target
                    } else {
                        resolved.join(target)
                    };
                    resolved = resolve_missing_path(&target, hops, false, false, reject_virtual)
                        .map_err(|error| {
                            if is_missing_path_error(&error, true) {
                                io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "broken symlink in path; cannot determine locality",
                                )
                            } else {
                                error
                            }
                        })?;
                    fs::metadata(&resolved)?.is_dir()
                } else {
                    resolved = candidate;
                    metadata.is_dir()
                };

                if components.peek().is_some() && !resolved_is_dir {
                    if allow_missing && allow_non_directory {
                        let mut suffix = Vec::new();
                        for component in components {
                            suffix.push(missing_component(component)?);
                        }
                        return rebuild_checked_missing_suffix(resolved, suffix, reject_virtual);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "non-directory ancestor in redirect path",
                    ));
                }
            }
        }
    }
    if reject_virtual {
        reject_unsafe_virtual_path(&resolved)?;
    }
    Ok(resolved)
}

fn is_unsafe_virtual_path(path: &Path) -> bool {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    [
        "/proc/self",
        "/proc/thread-self",
        "/dev/fd",
        "/dev/stdin",
        "/dev/stdout",
        "/dev/stderr",
        "/dev/tcp",
        "/dev/udp",
    ]
    .into_iter()
    .any(|prefix| normalized.starts_with(prefix))
}

fn render_path(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "redirect path is not valid UTF-8 and cannot be matched safely",
        )
    })
}

fn render_cwd_relative(path: &Path) -> io::Result<String> {
    if path.as_os_str().is_empty() {
        return Ok(".".to_string());
    }
    let rendered = render_path(path)?;
    if rendered.starts_with('~') || rendered.starts_with('&') {
        Ok(format!("./{rendered}"))
    } else {
        Ok(rendered.to_string())
    }
}

fn render_home_relative(path: &Path) -> io::Result<String> {
    if path.as_os_str().is_empty() {
        Ok("~".to_string())
    } else {
        Ok(format!("~/{}", render_path(path)?))
    }
}

pub(crate) fn canonicalize_allow_missing(path: &Path) -> io::Result<PathBuf> {
    let mut hops = 0;
    resolve_missing_path(path, &mut hops, true, true, false)
}

fn canonicalize_redirect_target(path: &Path) -> io::Result<PathBuf> {
    let mut hops = 0;
    resolve_missing_path(path, &mut hops, true, false, true)
}

fn rebuild_missing_suffix(
    mut base: PathBuf,
    components: impl IntoIterator<Item = MissingPathComponent>,
) -> io::Result<PathBuf> {
    // The canonicalized ancestor is the verified floor. A missing suffix may normalize within
    // itself, but it cannot climb above that ancestor through `..`.
    let floor = base.components().count();
    let mut depth = floor;

    for component in components {
        match component {
            MissingPathComponent::CurDir => {}
            MissingPathComponent::Normal(name) => {
                base.push(name);
                depth += 1;
            }
            MissingPathComponent::ParentDir => {
                if depth == floor {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "path escapes canonicalized ancestor",
                    ));
                }
                base.pop();
                depth -= 1;
            }
        }
    }

    Ok(base)
}

#[derive(Debug)]
enum MissingPathComponent {
    CurDir,
    ParentDir,
    Normal(OsString),
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };

    use tempfile::TempDir;

    use super::*;

    fn resolution_fixture<const N: usize>(names: [&str; N]) -> (TempDir, [PathBuf; N]) {
        let root = tempfile::tempdir().unwrap();
        let paths = names.map(|name| root.path().join(name));
        paths
            .iter()
            .for_each(|path| fs::create_dir_all(path).unwrap());
        (root, paths)
    }

    fn assert_resolution(
        context: &RedirectResolutionContext,
        target: &str,
        expand_home_tilde: bool,
        match_text: &str,
        is_local: bool,
    ) {
        let resolution = context.resolve(target, expand_home_tilde).unwrap();
        assert_eq!(resolution.match_text, match_text, "case {target:?}");
        assert_eq!(resolution.is_local, is_local, "case {target:?}");
    }

    #[test]
    fn windows_error_directory_is_recognized_on_all_test_hosts() {
        assert!(is_windows_error_directory(&io::Error::from_raw_os_error(
            267
        )));
    }

    #[test]
    fn redirect_target_resolution_matrix() {
        let (_root, [cwd, home, external]) = resolution_fixture(["project", "home", "external"]);
        let context = RedirectResolutionContext::new(&cwd, Some(&home));
        let external_target = fs::canonicalize(external).unwrap().join("out.txt");

        for (target, expand_home, match_text, is_local) in [
            ("reports/new.txt", false, "reports/new.txt", true),
            ("~", true, "~", false),
            ("~/.cache/tool/out", true, "~/.cache/tool/out", false),
            ("~//.cache/tool/out", true, "~/.cache/tool/out", false),
            ("~/.cache/tool/out", false, "./~/.cache/tool/out", true),
            ("&1", false, "./&1", true),
        ] {
            assert_resolution(&context, target, expand_home, match_text, is_local);
        }
        let external = context
            .resolve(external_target.to_str().unwrap(), false)
            .unwrap();
        assert_eq!(external.resolved_path, external_target);
        assert!(!external.is_local);
        for target in [
            "/proc/ignored/../self/cwd/out.txt",
            "/proc/thread-self/cwd/out.txt",
            "/dev/fd/1",
            "/dev/stdin",
            "/dev/stdout",
            "/dev/stderr",
            "/dev/tcp/attacker.example/80",
            "/dev/udp/attacker.example/53",
        ] {
            assert_eq!(
                context.resolve(target, false).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied,
                "case {target:?}"
            );
        }
        let no_home = RedirectResolutionContext::new(&cwd, None);
        assert!(no_home.resolve("~/out", true).is_err());
        let stale_home = RedirectResolutionContext::new(&cwd, Some(&home.join("missing")));
        assert_resolution(&stale_home, "local.txt", false, "local.txt", true);
        assert!(stale_home.resolve("~/out", true).is_err());
        let historical = RedirectResolutionContext::new(Path::new(""), None);
        assert!(historical.resolve("relative.txt", false).is_err());
        assert_resolution(&historical, "/dev/null", false, "/dev/null", false);
        let stale_cwd = RedirectResolutionContext::new(&cwd.join("missing"), None);
        assert!(stale_cwd.resolve("relative.txt", false).is_err());
        assert_resolution(&stale_cwd, "/dev/null", false, "/dev/null", false);
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_rejects_symlinks_to_process_relative_paths() {
        let (_root, [cwd]) = resolution_fixture(["project"]);
        symlink("/proc/self/fd/1", cwd.join("absolute-link")).unwrap();
        symlink("/proc/self/fd/1", cwd.join("process-link")).unwrap();
        symlink("process-link", cwd.join("relative-link")).unwrap();
        symlink("/proc", cwd.join("proc-link")).unwrap();
        symlink("proc-link/self/fd/2", cwd.join("outer")).unwrap();
        symlink("proc-link", cwd.join("nested-proc-link")).unwrap();
        let context = RedirectResolutionContext::new(&cwd, None);

        for target in [
            "absolute-link",
            "relative-link",
            "proc-link/self/fd/1",
            "outer",
            "nested-proc-link/self/cwd/out",
        ] {
            let error = context.resolve(target, false).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::PermissionDenied,
                "case {target:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_rejects_broken_symlinks() {
        let (_root, [cwd]) = resolution_fixture(["project"]);
        symlink("missing", cwd.join("broken")).unwrap();

        let error = RedirectResolutionContext::new(&cwd, None)
            .resolve("broken/out", false)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_rejects_symlink_chains_above_the_hop_limit() {
        let (_root, [cwd]) = resolution_fixture(["project"]);
        for index in 0..=MAX_SYMLINK_HOPS {
            symlink(
                format!("link_{}", index + 1),
                cwd.join(format!("link_{index}")),
            )
            .unwrap();
        }
        fs::write(cwd.join(format!("link_{}", MAX_SYMLINK_HOPS + 1)), "target").unwrap();

        let error = RedirectResolutionContext::new(&cwd, None)
            .resolve("link_0", false)
            .unwrap_err();
        assert!(error.to_string().contains("too many symbolic links"));
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_rejects_a_non_utf8_canonical_target() {
        let (root, [cwd]) = resolution_fixture(["project"]);
        let invalid = root.path().join(OsString::from_vec(b"x\xff".to_vec()));
        if fs::create_dir(&invalid).is_err() {
            assert!(render_path(&invalid).is_err());
            return;
        }
        symlink(&invalid, cwd.join("outside-link")).unwrap();
        let context = RedirectResolutionContext::new(&cwd, None);
        assert!(context.resolve("outside-link/out", false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_classifies_devices_inside_cwd_as_special() {
        let context = RedirectResolutionContext::new(Path::new("/dev"), None);
        let resolution = context.resolve("/dev/null", false).unwrap();
        assert!(resolution.is_local);
        assert!(resolution.is_device_or_special);
        assert_eq!(resolution.match_text, "/dev/null");

        let root = RedirectResolutionContext::new(Path::new("/"), None);
        assert!(!root.resolve("/dev/null", false).unwrap().is_local);
    }

    #[cfg(unix)]
    #[test]
    fn redirect_resolution_context_keeps_canonical_roots_stable() {
        let (root, [first, second, home_first, home_second]) =
            resolution_fixture(["first", "second", "home-first", "home-second"]);
        let [cwd_link, home_link] = ["cwd", "home"].map(|name| root.path().join(name));
        symlink(&first, &cwd_link).unwrap();
        symlink(&home_first, &home_link).unwrap();

        let context = RedirectResolutionContext::new(&cwd_link, Some(&home_link));
        let canonical_first = fs::canonicalize(&first).unwrap();
        let canonical_home_first = fs::canonicalize(&home_first).unwrap();
        context.resolve("~/cache", true).unwrap();
        fs::remove_file(&cwd_link).unwrap();
        fs::remove_file(&home_link).unwrap();
        symlink(&second, &cwd_link).unwrap();
        symlink(&home_second, &home_link).unwrap();

        let local = context.resolve("out.txt", false).unwrap();
        assert_eq!(local.resolved_path, canonical_first.join("out.txt"));
        assert!(local.is_local);
        let home = context.resolve("~/cache", true).unwrap();
        assert_eq!(home.resolved_path, canonical_home_first.join("cache"));
    }
}
