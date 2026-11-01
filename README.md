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

## GitHub sync and claims

```bash
punch project add cli --github cli/cli --scope all
punch sync -p cli
punch list -p cli --json
punch claim cli#123 --slot worker-1
punch unclaim cli#123
```

GitHub access uses the installed `gh` CLI and its authentication. The default
`assigned` scope pulls open issues assigned to you and open PRs authored by you;
`all` pulls every open issue and PR. Both refresh stored cards, including closed,
merged, and archived cards. Archived projects are skipped. Linked closing PRs
drive issue columns; new PR cards are suppressed when their issue is stored or
imported in the same sync. Previously imported PR cards retain their identity.
Sync replaces cached GitHub fields while keeping notes, priority, due dates,
dependencies, slot, ordering, and archive flags. A new fetch timestamp counts as
a change. Each project commits atomically; failures are reported alongside
successful projects, with exit 3 if any project fails.

For GitHub cards, claim/unclaim first adds/removes `@me` with `gh issue edit`,
then refreshes the card before committing locally. A remote or refresh failure
returns `offline` or `gh-failed`, preserving local fields. Local claims use the
actor label (`person` or the agent's identity) as assignee; `--slot` sets the
slot separately, defaulting to the agent identity. Unclaim clears the local
assignee and slot; GitHub assignees always come from the refreshed card.
