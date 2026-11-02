# punch

`punch` is a terminal punch list across every project one person runs, built
for working alongside coding agents. One local SQLite store, CLI first, then a
ratatui board and an MCP server. GitHub is the source of truth for issue and
PR state; Basecamp card tables are an optional mirror. "Done" for v1 is one
binary that shows every project's issues, PRs, and local tasks on one board,
lets agents claim and update work with `--json`, and keeps Basecamp in step.

## Status
v0.1.0 implementation is merged, including M6 board controls and release
packaging. Linux and Mac Studio artifact reviews passed; archive fixes #61/#62
verified independently. Release publication and tap validation are tracked in
#11. Binding design: `docs/features/v1/SPEC.md`; release: `docs/releasing.md`.

## Commands
```bash
cargo build
cargo run -- --help
cargo test --workspace
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
PUNCH_STATE_DIR=$(mktemp -d) cargo run -- list   # never touch the real store in dev
```

## Architecture
- `crates/punch-core`: store, migrations, model, `ops` (the only write path),
  GitHub and Basecamp adapters. No terminal dependencies.
- `crates/punch`: binary `punch`; `cli`, `tui`, `mcp` adapters that only call
  `punch_core::ops`.
- Full layout and rules: `docs/features/v1/SPEC.md` sections 3 and 13.

## Conventions & Gotchas
- Follow the spec; put open questions in the PR under "Spec questions"
  rather than redesigning.
- Test first per behaviour; every fix carries a test that fails without it.
- Set `PUNCH_STATE_DIR` in every test and dev run.
- Agent rules (no `done`, no archive) are enforced in `ops`, not in docs.
- Never write to a real GitHub repo or Basecamp project from tests or dev
  runs; use the trait fakes and fake CLIs on `PATH`.
- Commits: `<type>: <description>`, authored by @ContractorKeith, no AI
  attribution lines.

## Out of Scope
Launching agents, git/worktree/PR creation, Jira, web UI, time tracking,
two-way merge of GitHub fields, offline write queues, Windows. See SPEC
section 2.
