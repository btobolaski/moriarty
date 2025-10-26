# Moriarty, an assistant for an assistant

A Rust CLI tool for analyzing Claude Code logs and API usage.

## Features

- **Logs viewer**: An interactive TUI for viewing and navigating Claude Code conversation logs
- **API pricing analyzer**: Analyzes Claude API usage from log directories and generates detailed cost reports
- **MCP servers**: Provides Model Context Protocol servers for git operations and project tools

## Requirements

### Testing

**⚠️ Important**: Tests MUST be run using `cargo nextest`:

```bash
cargo nextest run
```

Do **NOT** use `cargo test` as tests use `std::env::set_var` to set up isolated XDG config directories, which is only safe when each test runs in its own process. `cargo nextest` runs each test in a separate process, making this safe and preventing tests from clobbering your real config files.

To install `cargo nextest`:

```bash
cargo install cargo-nextest
```

## Development

See [CLAUDE.md](./CLAUDE.md) for detailed development instructions.
