# Plan: Add Man Pages for Moriarty

## Context

Moriarty is a Rust CLI tool for Claude Code log analysis, API pricing, MCP servers, and tool call permissioning. It currently has no man pages. The goal is to add man pages covering moriarty's operation — especially the tool/bash rules system — and integrate them into the Nix build so `man moriarty` works when installed via Nix.

## Approach

Write man pages in **mdoc(7)** format (the standard BSD/Linux man page markup) and install them via the Nix derivation using `postInstall` in `flake.nix`. This keeps man page sources in the repo as plain text files and uses the standard Nix pattern of copying them into `$out/share/man/manN/`.

### Man Page Structure

Three man pages covering the main areas:

| Page | Section | Content |
|------|---------|---------|
| `moriarty(1)` | 1 (commands) | CLI overview, subcommands (logs, api-pricing, mcp, hooks, test, approve-project), general usage |
| `moriarty-tool-rules(5)` | 5 (file formats) | Tool rules configuration: `~/.config/moriarty/tool_rules.toml` format, `[[tool_rules]]` section, field/pattern matching, CWD prefix stripping, evaluation order |
| `moriarty-bash-rules(5)` | 5 (file formats) | Bash rules configuration: `[[bash_rules]]` section, actions (Allow/Deny/Modify/Ask/ArgumentFilter), pattern fragments, security best practices |

Section 5 is appropriate for tool_rules and bash_rules since they document a configuration file format (`tool_rules.toml`).

### File Layout

```
doc/man/
  moriarty.1
  moriarty-tool-rules.5
  moriarty-bash-rules.5
```

### Nix Integration

Modify the `moriarty` derivation in `flake.nix` to add a `postInstall` phase that copies man pages into `$out/share/man/`. The man page source files need to be included in the `fileSetForCrate` so crane picks them up.

## Files to Modify

- **`flake.nix`** — Add `postInstall` to the `moriarty` derivation to install man pages; update `fileSetForCrate` to include `doc/man/`
- **`doc/man/moriarty.1`** — New file: main man page (section 1)
- **`doc/man/moriarty-tool-rules.5`** — New file: tool rules config format (section 5)
- **`doc/man/moriarty-bash-rules.5`** — New file: bash rules config format (section 5)

## Reuse

- Content for tool/bash rules pages comes directly from `BASH_RULES.md` (comprehensive documentation already exists)
- CLI structure/flags from `crates/moriarty/src/main.rs` (clap definitions)

## Steps

- [ ] Create `doc/man/moriarty.1` — mdoc format man page covering CLI synopsis, subcommands, description, environment, files, and see-also references
- [ ] Create `doc/man/moriarty-tool-rules.5` — mdoc format man page for `[[tool_rules]]` config section
- [ ] Create `doc/man/moriarty-bash-rules.5` — mdoc format man page for `[[bash_rules]]` config section, including pattern fragments and all action types
- [ ] Update `flake.nix`:
  - Add `doc/man` directory to `fileSetForCrate` via `lib.fileset.unions`
  - Add `pkgs.installShellFiles` to `nativeBuildInputs` of the `moriarty` derivation (this hook provides the `installManPage` shell function)
  - Add `postInstall` to the `moriarty` derivation to install man pages:
    ```nix
    nativeBuildInputs = [ pkgs.installShellFiles ];
    postInstall = ''
      installManPage doc/man/moriarty.1
      installManPage doc/man/moriarty-tool-rules.5
      installManPage doc/man/moriarty-bash-rules.5
    '';
    ```
    `installManPage` (from `installShellFiles`) automatically places pages in `$out/share/man/manN/` based on the file extension.
  - The `src` for the derivation needs the `doc/man` directory. Add `./doc/man` to the `fileSetForCrate` unions, or add it directly to the moriarty derivation's `src` override.

## Verification

- `nix build .#moriarty` succeeds
- `ls result/share/man/man1/moriarty.1` and `ls result/share/man/man5/moriarty-*.5` exist
- `MANPATH=result/share/man man moriarty` renders correctly
- `MANPATH=result/share/man man moriarty-tool-rules` renders correctly
- `MANPATH=result/share/man man moriarty-bash-rules` renders correctly
- Man pages pass `mandoc -Tlint` with no errors (if mandoc is available)
