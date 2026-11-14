# Changelog

All notable changes to this project are documented here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Workspace roots (pnpm + cargo), three crates with fixed boundaries, Tauri 2 entry point.
- `scripts/fetch-kernel.mjs` — installs the pinned `@deepseek-ai/dsh` into `kernel/`,
  verifies the resolved version, and swaps it in atomically.
- `scripts/smoke-m0.mjs` — spawns the kernel in its `sdk` profile, sends `initialize`,
  asserts the reply, and shuts down. This is the M0 acceptance gate and needs no API key.
- `scripts/gen-icons.mjs` — writes the PNG/ICO icon set with no image dependency.
- Profile patch that turns off zstd session-log compression so history can be read as NDJSON.
- Docs: design decisions, engineering notes, status, and REQ-001.
