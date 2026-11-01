# punch

A terminal punch list across every project, for you and your coding agents.
GitHub issues and PRs, plus local tasks, on one board. GitHub is the source of
truth; Basecamp card tables can mirror it.

Status: v1 in development. Design: [docs/features/v1/SPEC.md](docs/features/v1/SPEC.md).

License: MIT.

## Usage

The CLI and board support local tasks, GitHub sync, claims, dependencies, and
agent-safe writes.

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
Changes advance the task revision; an exhausted revision refuses the change
with `storage-failed` (exit 3), leaving the task unchanged.
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
temporary directory.

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
dependencies, slot, ordering, and archive flags.
A fetched_at-only refresh reports `changed: false` and keeps task revisions.
Each project commits atomically; failures are reported alongside
successful projects, with exit 3 if any project fails. GitHub fetching completes
before the short local write transaction, so reads and local writes keep working
during a slow fetch. Each `gh` subprocess has a 30-second timeout (`gh-failed`).
GraphQL connection walks reject repeated cursors and stop after 100 pages.

Malformed PR or closing-reference numbers fail sync with `gh-failed`, preserving
the project's cached cards and links. Empty non-PR cross-references are skipped.
PRs require a boolean draft state and a review decision of null, `APPROVED`,
`CHANGES_REQUESTED`, or `REVIEW_REQUIRED`; invalid values also fail before import.

For GitHub cards, claim/unclaim first adds/removes `@me` with `gh issue edit`,
then refreshes the card before committing locally. A remote or refresh failure
returns `offline` or `gh-failed`, preserving local fields. Local claims use the
actor label (`person` or the agent's identity) as assignee; `--slot` sets the
slot separately, defaulting to the agent identity. Unclaim clears the local
assignee and slot; GitHub assignees always come from the refreshed card.

## MCP and agent guide

```bash
punch guide
punch mcp --as worker-1
punch setup claude
punch setup codex
```

`mcp` serves newline-delimited JSON-RPC on stdin/stdout synchronously. It exposes
17 tools for the available CLI operations, including `guide`, `project_add`,
`project_list`, `project_archive`, and `add_plan` (the `add --file` operation).
Tools share CLI validation and JSON envelopes. Results include the envelope as
both `structuredContent` and JSON text in `content`; refusals and partial
failures set `isError: true`. No stdout logging is emitted.

MCP always acts as an agent: `--as` overrides `PUNCH_AGENT`, then defaults to
`mcp`. Tools cannot override that identity; `claim`'s `slot` argument controls
the claimed slot separately. Tool fields use long CLI flag names (including
`clear-due` and `clear-after`), positional argument names such as `reference`,
and JSON arrays for `after`. `add_plan` accepts a `file` path; `-` is refused
because stdin belongs to the transport. Paths must resolve through symlinks to
regular files; devices, FIFOs, and stdin aliases are refused before opening.
Request payloads (excluding LF or CRLF) and plan files are capped at 1 MiB.
Oversized lines return Invalid Request; oversized plans return `invalid-input`.
Serialized tool results, including both text and structured content and JSON
escaping, are capped at 4 MiB. Larger results return an `isError: true`
`invalid-input` refusal: narrow the request with a project filter, a limit,
or show a single task. The server continues with the next request. Basecamp
tools arrive in phase 5.

Request ids must be strings or integers fitting i64 or u64 and are echoed
exactly. Null, fractional, and out-of-range ids return Invalid Request with a
null response id. Supported protocol versions are `2025-06-18` and `2025-11-25`;
older or unknown versions negotiate `2025-06-18`. Batches are not supported.

`guide` prints the embedded agent instructions. `setup` only prints the exact
registration command for Claude Code or Codex; it does not execute it, open the
store, or change configuration. Both also support `--json` envelopes.

## Board

Bare `punch` and `punch board` open the interactive board in a terminal.
Use `h/l` for columns, `j/k` for cards, `p` for projects, `/` for text filtering,
`Enter` for details, `?` for help, and `q` to quit. Ctrl-C quits from any mode
and restores the terminal. Narrow terminals show the
focused column and as many neighbours as fit. External CLI changes reload once
a second. `--json` remains noninteractive; agents should use `list --json`.

Move the selected card with `H/L` or `Space/Backspace`, cycle its priority with
`P`, add with `a`, edit notes with `e` (set `$EDITOR`), and archive with `x`
then `y`. Adding from “all” first asks for a project; new cards start in Todo.
Notes edits and archive confirmations retain the original revision, so an
external update produces `stale` and reloads instead of overwriting it. Other
refusals and successful-action warnings flash for five seconds. `j/k` scroll long detail and help panes;
`Esc` closes a pane or cancels input.

$EDITOR must wait for the file to close before returning. For GUI editors, use
`EDITOR='open -W -t'` on macOS, `EDITOR='code --wait'`, or `EDITOR='subl -w'`.
If the editor returns with unchanged notes, the board flashes that no change
was made and reminds you of the wait requirement.
