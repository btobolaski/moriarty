//! Persistence layer for Moriarty configuration and application state.
//!
//! This module provides XDG Base Directory Specification-compliant file storage
//! for application data. Files are stored in appropriate XDG directories:
//! - Config files: `$XDG_CONFIG_HOME/moriarty/` (typically `~/.config/moriarty/`)
//!
//! All data is serialized to TOML format for human-readable configuration files.
//!
//! # Example
//!
//! ```no_run
//! use moriarty::persistence::FileType;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct UserPreferences {
//!     theme: String,
//!     auto_save: bool,
//! }
//!
//! # async fn example() -> miette::Result<()> {
//! // Save preferences
//! let prefs = UserPreferences {
//!     theme: "dark".into(),
//!     auto_save: true,
//! };
//! FileType::Config.persist("preferences.toml", &prefs).await?;
//!
//! // Load preferences later
//! let loaded: UserPreferences = FileType::Config.load("preferences.toml").await?;
//! assert_eq!(loaded.theme, "dark");
//! # Ok(())
//! # }
//! ```
//!
//! # XDG Directory Structure
//!
//! The module follows the XDG Base Directory Specification, creating directories
//! as needed. On Linux systems, config files are stored at:
//! - `$XDG_CONFIG_HOME/moriarty/` if `XDG_CONFIG_HOME` is set
//! - `~/.config/moriarty/` otherwise
//!
//! Future FileType variants will support:
//! - Data files (persistent application data)
//! - Cache files (expendable cached data)
//! - State files (application state, logs, etc.)

use std::path::PathBuf;

use miette::{IntoDiagnostic, WrapErr};
use serde::{de::DeserializeOwned, Serialize};
use tokio::task::spawn_blocking;

/// The type of file being stored
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileType {
    /// Config files are for user configuration. They can be modified at runtime so, they are not
    /// just for immutable configuration.
    Config,
    /// State files are for application state, logs, and runtime data.
    State,
}

impl FileType {
    const APP_PREFIX: &str = "moriarty";

    /// Builds a PathBuf for a file of the particular type with the given name.
    ///
    /// This method follows the XDG Base Directory specification and creates parent
    /// directories as needed. The file path is constructed based on the FileType:
    /// - `Config`: `$XDG_CONFIG_HOME/moriarty/{file_name}`
    /// - `State`: `$XDG_STATE_HOME/moriarty/{file_name}`
    ///
    /// # Arguments
    ///
    /// * `file_name` - The name of the file. Must have `'static` lifetime because this value is
    ///   moved into `spawn_blocking` which executes on a separate thread pool. The `'static`
    ///   bound ensures the string outlives the spawned task.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation fails or if there are threading issues.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use moriarty::persistence::FileType;
    /// # async fn example() -> miette::Result<()> {
    /// let path = FileType::Config.build_path("settings.toml").await?;
    /// println!("Config will be stored at: {:?}", path);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build_path(&self, file_name: &'static str) -> miette::Result<PathBuf> {
        let file_type = *self; // Copy the enum value to move into the closure
        let result = spawn_blocking(move || {
            let xdg_dirs = xdg::BaseDirectories::with_prefix(Self::APP_PREFIX);

            match file_type {
                Self::Config => xdg_dirs.place_config_file(file_name).into_diagnostic(),
                Self::State => xdg_dirs.place_state_file(file_name).into_diagnostic(),
            }
        });

        result.await.map_err(|error| {
            miette::Error::from_err(error).context("threading error while building path")
        })?
    }

    /// Serializes data to TOML and writes it to a file in the XDG directory structure.
    ///
    /// This method creates parent directories as needed, serializes the provided data
    /// to pretty-printed TOML format, and writes it to the appropriate XDG location.
    /// If the file already exists, it will be overwritten.
    ///
    /// # Arguments
    ///
    /// * `file_name` - The name of the file. Must have `'static` lifetime because this value is
    ///   moved into `spawn_blocking` which executes on a separate thread pool. The `'static`
    ///   bound ensures the string outlives the spawned task.
    /// * `contents` - The data to serialize and persist (must implement `Serialize`)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Directory creation fails
    /// - Serialization to TOML fails
    /// - File writing fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use moriarty::persistence::FileType;
    /// # use serde::Serialize;
    /// #[derive(Serialize)]
    /// struct Config {
    ///     setting: String,
    /// }
    ///
    /// # async fn example() -> miette::Result<()> {
    /// let config = Config { setting: "value".into() };
    /// FileType::Config.persist("app.toml", &config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn persist<T>(&self, file_name: &'static str, contents: T) -> miette::Result<()>
    where
        T: Serialize,
    {
        let path = self
            .build_path(file_name)
            .await
            .context("failed to build path for persistence")?;
        let serialized = toml::to_string_pretty(&contents)
            .into_diagnostic()
            .context("failed to serialize value")?;
        tokio::fs::write(path, serialized)
            .await
            .into_diagnostic()
            .context("failed to write file")
    }

    /// Reads a file from the XDG directory structure and deserializes it from TOML.
    ///
    /// This method reads the file at the appropriate XDG location and deserializes
    /// its TOML contents into the specified type.
    ///
    /// # Arguments
    ///
    /// * `file_name` - The name of the file. Must have `'static` lifetime because this value is
    ///   moved into `spawn_blocking` which executes on a separate thread pool. The `'static`
    ///   bound ensures the string outlives the spawned task.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Directory path construction fails
    /// - File does not exist or cannot be read
    /// - File contents are not valid TOML
    /// - Deserialization fails (type mismatch, missing fields, etc.)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use moriarty::persistence::FileType;
    /// # use serde::Deserialize;
    /// #[derive(Deserialize)]
    /// struct Config {
    ///     setting: String,
    /// }
    ///
    /// # async fn example() -> miette::Result<()> {
    /// let config: Config = FileType::Config.load("app.toml").await?;
    /// println!("Setting: {}", config.setting);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn load<T>(&self, file_name: &'static str) -> miette::Result<T>
    where
        T: DeserializeOwned,
    {
        let path = self
            .build_path(file_name)
            .await
            .context("failed to build path for loading")?;
        let file_contents = tokio::fs::read(&path)
            .await
            .into_diagnostic()
            .context("failed to read from file")?;
        toml::from_slice(file_contents.as_slice())
            .into_diagnostic()
            .context("failed to parse file contents")
    }
}

#[cfg(test)]
mod tests {
    //! Test isolation strategy: These tests modify XDG_CONFIG_HOME environment variables,
    //! requiring separate processes to avoid race conditions. Use `cargo nextest run` which
    //! provides process isolation; `cargo test` uses thread-level isolation and will fail.

    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestConfig {
        value: String,
        count: i32,
    }

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_config() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        temp_dir
    }

    #[tokio::test]
    async fn test_build_path_creates_valid_path() {
        let _xdg_dir = setup_isolated_xdg_config();

        let path = FileType::Config
            .build_path("test_build_path.toml")
            .await
            .unwrap();

        assert!(path.to_string_lossy().contains("moriarty"));
        assert!(path.to_string_lossy().ends_with("test_build_path.toml"));
    }

    #[tokio::test]
    async fn test_build_path_creates_directories() {
        let _xdg_dir = setup_isolated_xdg_config();

        let path = FileType::Config
            .build_path("test_build_dirs.toml")
            .await
            .unwrap();

        assert!(path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn test_persist_writes_valid_toml() {
        let _xdg_dir = setup_isolated_xdg_config();

        let config = TestConfig {
            value: "test".to_string(),
            count: 42,
        };

        FileType::Config
            .persist("test_persist_writes.toml", &config)
            .await
            .unwrap();

        let path = FileType::Config
            .build_path("test_persist_writes.toml")
            .await
            .unwrap();
        assert!(path.exists());

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("value = \"test\""));
        assert!(contents.contains("count = 42"));
    }

    #[tokio::test]
    async fn test_load_reads_valid_toml() {
        let _xdg_dir = setup_isolated_xdg_config();

        let original = TestConfig {
            value: "test".to_string(),
            count: 42,
        };

        FileType::Config
            .persist("test_load_reads.toml", &original)
            .await
            .unwrap();

        let loaded: TestConfig = FileType::Config.load("test_load_reads.toml").await.unwrap();
        assert_eq!(loaded, original);
    }

    #[tokio::test]
    async fn test_persist_overwrites_existing_file() {
        let _xdg_dir = setup_isolated_xdg_config();

        let config1 = TestConfig {
            value: "first".to_string(),
            count: 1,
        };
        let config2 = TestConfig {
            value: "second".to_string(),
            count: 2,
        };

        FileType::Config
            .persist("test_overwrites.toml", &config1)
            .await
            .unwrap();
        FileType::Config
            .persist("test_overwrites.toml", &config2)
            .await
            .unwrap();

        let loaded: TestConfig = FileType::Config.load("test_overwrites.toml").await.unwrap();
        assert_eq!(loaded, config2);
    }

    #[tokio::test]
    async fn test_load_nonexistent_file_returns_error() {
        let _xdg_dir = setup_isolated_xdg_config();

        let err = FileType::Config
            .load::<TestConfig>("nonexistent.toml")
            .await
            .expect_err("Should fail with nonexistent file");
        assert!(err.to_string().contains("failed to read from file"));
    }

    #[tokio::test]
    async fn test_load_malformed_toml_returns_error() {
        let _xdg_dir = setup_isolated_xdg_config();

        let path = FileType::Config.build_path("bad.toml").await.unwrap();
        tokio::fs::write(&path, "not valid toml {[}").await.unwrap();

        let err = FileType::Config
            .load::<TestConfig>("bad.toml")
            .await
            .expect_err("Should fail with malformed TOML");
        assert!(err.to_string().contains("failed to parse file contents"));
    }

    #[tokio::test]
    async fn test_persist_empty_string() {
        let _xdg_dir = setup_isolated_xdg_config();

        let config = TestConfig {
            value: "".to_string(),
            count: 0,
        };

        FileType::Config
            .persist("test_empty_string.toml", &config)
            .await
            .unwrap();
        let loaded: TestConfig = FileType::Config
            .load("test_empty_string.toml")
            .await
            .unwrap();
        assert_eq!(loaded.value, "");
    }

    #[tokio::test]
    async fn test_persist_special_characters() {
        let _xdg_dir = setup_isolated_xdg_config();

        let config = TestConfig {
            value: "quotes\"and'newlines\nand\ttabs".to_string(),
            count: 123,
        };

        FileType::Config
            .persist("test_special_chars.toml", &config)
            .await
            .unwrap();
        let loaded: TestConfig = FileType::Config
            .load("test_special_chars.toml")
            .await
            .unwrap();
        assert_eq!(loaded, config);
    }

    #[tokio::test]
    async fn test_persist_unicode() {
        let _xdg_dir = setup_isolated_xdg_config();

        let config = TestConfig {
            value: "日本語 🦀 Émojis".to_string(),
            count: 999,
        };

        FileType::Config
            .persist("test_unicode.toml", &config)
            .await
            .unwrap();
        let loaded: TestConfig = FileType::Config.load("test_unicode.toml").await.unwrap();
        assert_eq!(loaded, config);
    }

    #[tokio::test]
    async fn test_round_trip_preserves_data() {
        let _xdg_dir = setup_isolated_xdg_config();

        let test_cases = [
            (
                TestConfig {
                    value: "simple".to_string(),
                    count: 1,
                },
                "roundtrip_0.toml",
            ),
            (
                TestConfig {
                    value: "with\nnewlines\nand\ttabs".to_string(),
                    count: -42,
                },
                "roundtrip_1.toml",
            ),
            (
                TestConfig {
                    value: "Unicode: 你好世界 🌍".to_string(),
                    count: i32::MAX,
                },
                "roundtrip_2.toml",
            ),
        ];

        for (i, (original, filename)) in test_cases.iter().enumerate() {
            FileType::Config.persist(filename, original).await.unwrap();
            let loaded: TestConfig = FileType::Config.load(filename).await.unwrap();
            assert_eq!(&loaded, original, "Round-trip failed for config {}", i);
        }
    }
}
