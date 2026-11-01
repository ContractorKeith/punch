# punch

A terminal punch list across every project, for you and your coding agents.
GitHub issues and PRs, plus local tasks, on one board. GitHub is the source of
truth; Basecamp card tables can mirror it.

Status: v1 in development. Design: [docs/features/v1/SPEC.md](docs/features/v1/SPEC.md).

License: MIT.

## Usage

The local store and CLI support projects, tasks, dependencies, and agent-safe writes.

```bash
punch project add demo --name "Demo project"
punch add "Build the CLI" -p demo --priority 1 --due 2026-12-01
punch add "Write usage docs" -p demo --after t1
punch ready -p demo
punch move t1 doing
punch note t1 "Implementation started"
punch show t1
punch list --all
```

Agents should pass `--json` and identify themselves with `--as <slot>` or
`PUNCH_AGENT=<slot>`. Ops enforce agent permissions, including no finishing or
archiving cards. Writes report `changed`; identical adds return `existing: true`.
Notes append a timestamp and actor on every call and are not idempotent.
Task references accept `t12`, `12`, or `project#566` for cached GitHub cards.

Import a JSON array with `punch add --file plan.json` (or `--file -` for stdin):

```json
[
  {"project":"demo","title":"First task","priority":1},
  {"project":"demo","title":"Second task","after":["t1"]}
]
```

Each plan item reports created, existing, or failed. Valid items commit in one
transaction; validation failures exit 1. See [refusal codes and output contract](docs/refusal-codes.md).

Reads use deferred transactions, so they keep working while another process
holds the WAL write lock. Mutations serialize in immediate transactions.
`show` loads one task; `ready --limit N` loads full details only for its results.

The store lives in `$PUNCH_STATE_DIR/punch.db`, otherwise the platform data
directory under `punch/`. For development, always set `PUNCH_STATE_DIR` to a
temporary directory. GitHub synchronization, MCP, and Basecamp integration are planned.

## Board

Bare `punch` and `punch board` open the interactive board in a terminal.
Use `h/l` for columns, `j/k` for cards, `p` for projects, `/` for text filtering,
`Enter` for details, `?` for help, and `q` to quit. Narrow terminals show the
focused column and as many neighbours as fit. External CLI changes reload once
a second. `--json` remains noninteractive; agents should use `list --json`.

Move the selected card with `H/L` or `Space/Backspace`, cycle its priority with
`P`, add with `a`, edit notes with `e` (set `$EDITOR`), and archive with `x`
then `y`. Adding from “all” first asks for a project; new cards start in Todo.
Notes edits and archive confirmations retain the original revision, so an
external update produces `stale` and reloads instead of overwriting it. Other
refusals flash for five seconds. `j/k` scroll long detail and help panes;
`Esc` closes a pane or cancels input.
