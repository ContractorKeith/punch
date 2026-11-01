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

## Basecamp mirror

Link a local project to an existing Basecamp card table (person only):

```bash
punch bc link -p demo --bucket 123 --card-table 456 --map todo=10,doing=20,review=30,blocked=40,done=50,shipped=60
punch bc status -p demo --json
punch sync -p demo
```

Basecamp access uses the installed `basecamp` CLI (0.21.0) and its authentication.
Linking saves configuration locally; the next sync creates cards for existing
tasks. New tasks and changes to title, column, or slot mirror automatically.
Titles are `#7 · worker-1 · Title` for GitHub cards and `t12 · Title` for local
tasks, omitting empty slots. Unmapped columns produce `basecamp-unmapped` warnings.

The local task and dirty mirror intent commit before any Basecamp call; no
transaction stays open while the CLI runs. Each call has a 30-second timeout.
Failures produce `basecamp-failed` warnings while the task operation succeeds.
Credentials in diagnostics are redacted before warnings or storage, and each
diagnostic is limited to 2 KiB. Each captured stream is capped at 1 MiB; excess
output is drained and reported as truncated. A timeout kills the process group
and includes stderr received before the deadline.
Status shows mapped columns, dirty cards, and their last errors. Failed creates
have an empty card ID until retried. A store claim serializes creation across
processes; after a crash, sync can retry once the five-minute claim expires.
A late creator preserves a newer stored card ID and trashes its duplicate.
Before creating, punch lists the complete
card table and adopts an exact-title match, recovering a remote create that
succeeded before punch crashed. Sync reconciles all non-archived tasks in
linked, active projects after the GitHub pull, including local-only projects;
it retries failures without bumping task revisions. A fetch timestamp alone
keeps `changed=false` and preserves revisions. A changed mirror record (such as
a successful retry) sets the overall sync `changed=true`; the per-project
results continue to describe GitHub imports.

Relinking to another bucket or card table clears every local card association,
including archived tasks, because old card IDs belong to the old table. Archived
tasks receive no remote calls; the next sync mirrors active tasks at the new destination. Existing remote cards are left
in place. Changing only the column map retains card IDs.

## MCP and agent guide

```bash
punch guide
punch mcp --as worker-1
punch setup claude
punch setup codex
```

`mcp` serves newline-delimited JSON-RPC on stdin/stdout synchronously. It exposes
19 tools for the available CLI operations, including `guide`, `project_add`,
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
escaping, are capped at 4 MiB. A committed write (`changed: true`) returns a
small successful identity receipt (task ref and revision for task operations)
with `result-truncated: result exceeded 4 MiB; use show to read it`.
Larger read results return an `isError: true`
`invalid-input` refusal: narrow the request with a project filter, a limit,
or show a single task. The server continues with the next request.
`bc_status` reads mirror status; `bc_link` is exposed for parity but core ops
refuse every MCP agent with `agent-cannot-manage-projects` because linking is
person-only.

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
