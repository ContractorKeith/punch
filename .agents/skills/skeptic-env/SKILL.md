---
name: skeptic-env
description: "How a skeptical reviewer builds, runs, seeds and attacks punch. Read by the skeptic-review skill; not a workflow on its own."
---

# skeptic-env — punch

`punch` is a Rust terminal punch list for one person and their coding agents:
one SQLite store, a CLI with `--json`, a ratatui board, a stdio MCP server,
GitHub as the source of truth, and an optional Basecamp card-table mirror. The
binding design is `docs/features/v1/SPEC.md`; "done" for a PR is its SPEC
sections plus its milestone issue.

## Stack and commands

```bash
cargo build
cargo test --workspace
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings   # as CI runs it
export PUNCH_STATE_DIR=$(mktemp -d)     # every manual run; never the real store
cargo run -q -- project add demo --name Demo
cargo run -q -- add "First card" -p demo --json
cargo run -q -- list --json
cargo run -q -- board                    # TUI, phase 2 and later
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | cargo run -q -- mcp   # phase 4 and later
```

Scratch data: `PUNCH_STATE_DIR`. The real store is `~/.local/share/punch` on
Linux and `~/Library/Application Support/punch` on macOS; never touch it.
GitHub: read-only `gh` calls against public repos only (for example
`cli/cli`); never `punch claim` against a real repo, never any write to a real
repo. Basecamp: never point `punch bc link` at a real project; use a fake
`basecamp` script first on `PATH`.

## Platforms

| Platform | Ships | Review on | Notes |
|---|---|---|---|
| Linux | yes | Omarchy | CI runs ubuntu-latest |
| macOS | yes | Mac Studio | CI runs macos-latest; data dir differs |
| Windows | no | not reviewed | out of scope for v1 |

## What to attack

- **Output contract:** every command with `--json` prints exactly one JSON
  object and nothing else; exit codes 0/1/2/3 match SPEC §8; refusal codes
  match `docs/refusal-codes.md`; usage errors under `--json` are still JSON.
- **Agent rules:** with `PUNCH_AGENT=x` or `--as x`, try every forbidden
  action in SPEC §6 through the CLI, the TUI and MCP. Any path that lets an
  agent finish, archive, manage projects or link Basecamp is high severity.
- **Data:** idempotent add twice; plans with mixed valid and invalid items;
  dependency cycles across several hops; refs `t0`, `t-1`, huge ids, `slug#`,
  unicode titles, empty and very long titles and notes; due dates in wrong
  formats; two processes writing at once (`busy_timeout`, `BEGIN IMMEDIATE`).
- **Store:** a newer `schema_version` refuses with `store-too-new`; a corrupt
  or zero-byte `punch.db`; `PUNCH_STATE_DIR` that does not exist, is a file, or
  is read-only gives a sentence, not a panic.
- **GitHub:** derived columns per SPEC §5 in list, show and ready; sync twice
  changes nothing the second time; `gh` missing, unauthenticated, or failing
  mid-sync; nothing local changes when a write-through fails.
- **TUI:** tiny and huge terminals, resize while running, every key in SPEC
  §12, an external CLI write showing up live, a stale edit refused, `$EDITOR`
  unset or failing, the terminal restored after a panic or Ctrl-C.
- **MCP:** malformed JSON, unknown method, missing params, huge input, a
  closed stdin mid-call, stdout carrying anything but JSON-RPC.
- **Basecamp:** the fake CLI failing, slow or returning junk; dirty cards
  retried on sync; archived tasks left alone; how many CLI calls one sync makes.

## Traps

- PRs are stacked: #1 on main, #3 and #2 on #1, #4 and #5 on #2. Judge each
  PR's own diff against its base, not against main.
- GitHub cards cannot be moved by hand by anyone; `column-is-derived` is
  intended.
- Agents cannot replace titles or notes (`agent-cannot-edit`), only append
  notes: a recorded phase 1 interpretation, not a bug.
- `note` is intentionally not idempotent.
- Partial plan and partial sync results use `ok:true` with per-item results
  and exit 1 or 3: recorded interpretations in PRs #1 and #2.

## Earlier rounds

- 2026-11-01, round 1, Linux only: PRs #1 `3cc8e23`, #3 `f302e3f`, #2 `942ca1e`, #4 `5699af1`, #5 `13b1f9e`. Report `worklog/reports/punch-review-2026-11-01.md`. Issues #12–#39 (`skeptic`), all fixed on their PR branches (heads #1 0a92960, #3 f181839, #2 2e9b9b1, #4 6cf7f1d, #5 db27d69). Re-check them first; the stack is now linear, main <- #1 <- #3 <- #2 <- #4 <- #5.
- 2026-11-01, round 2, Linux per PR plus first macOS pass (Mac Studio) on the #5 head: issues #40–#56; fix verification on `0806ec8` found #57 and #58. All fixed and merged to main at `40ed11f`. Reports `worklog/reports/punch-review-2026-11-01-r2.md`. Next round: target main, start by re-checking #53/#57 redaction and #54 concurrent mirrors.
