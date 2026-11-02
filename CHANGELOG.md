# Changelog

## 0.1.0 — 2026-11-02

### Added

- Local tasks, dependencies, priorities, due dates, notes, and agent-safe claims
  in a single SQLite store, with CLI JSON output.
- Terminal board, GitHub issue and PR sync, optional Basecamp card mirror,
  and a stdio MCP server for coding agents.
- Release packaging for Linux x86_64 musl and macOS Apple Silicon and Intel,
  with SHA-256 checksums and a generated Homebrew formula.
- Board claim/unclaim and background sync, persistent action results, and
  bounded subprocess cancellation that preserves process-group identity.

### Fixed

- Release validation rejects damaged gzip streams before metadata generation
  or executable extraction and reports concise archive diagnostics.
