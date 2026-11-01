# punch

A terminal punch list across every project, for you and your coding agents.
GitHub issues and PRs, plus local tasks, on one board. GitHub is the source of
truth; Basecamp card tables can mirror it.

Status: v1 in development. Design: [docs/features/v1/SPEC.md](docs/features/v1/SPEC.md).

License: MIT.

## Usage

Phase 1 provides the local store and CLI. Bare `punch` prints help.

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

The store lives in `$PUNCH_STATE_DIR/punch.db`, otherwise the platform data
directory under `punch/`. For development, always set `PUNCH_STATE_DIR` to a
temporary directory. GitHub synchronization, the board, MCP, and Basecamp
integration arrive in later phases.
