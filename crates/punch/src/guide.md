# Working with punch

Always pass --json to punch CLI commands. Identify yourself with --as <slot>
or PUNCH_AGENT=<slot>. MCP calls always use an agent identity and return the
same JSON envelope; the default MCP slot is mcp.

Start each session with `punch sync --json`, then `punch ready --json`.
Read a card with `punch show <ref> --json`, and claim before starting:
`punch claim <ref> --json`. For local cards, move to doing while working and
move to review when the PR is ready: `punch move <ref> review --json`.
GitHub card columns are derived from GitHub state; sync to refresh them.
Append progress with `punch note <ref> "progress" --json`.

Agents must never try to finish (done or shipped) or archive/unarchive cards,
manage projects, or link Basecamp. Leave finishing and archiving to a person.
Agents may append notes and edit priority, due date, and dependencies; they
cannot replace titles or notes. Use unclaim when giving up a claim.

Read ok and changed in success envelopes. changed=false is a successful no-op.
On failure, read each entry in refusals: code is a stable kebab-case reason,
message explains it, and ref identifies the affected item. Fix the cause;
do not retry a permission refusal with a different identity. Examples:
agent-cannot-finish means hand off to a person; column-is-derived means sync;
after-not-done means finish the dependencies first through the proper owner.
See docs/refusal-codes.md for the full code list.

CLI exit codes: 0 success (including no-op); 1 refusal; 2 usage error;
3 storage or external-tool failure (a write may have happened: read back
before retrying). Plans can have created/existing/failed items with exit 1;
sync can have successful and failed projects with exit 3 even with ok=true.
MCP marks such failures with isError=true; inspect the envelope and item codes.

Writes are idempotent except note: note is not idempotent, and every retry
appends another timestamped entry. Do not blindly replay notes. For add,
the same project, title, and assignee returns the existing active local task.
