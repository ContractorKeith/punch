# Refusal codes

Stable, kebab-case codes returned with exit 1 (or exit 3 where noted). Never
renamed once released. Each phase adds its codes here in the same PR.

| code | meaning |
|---|---|
| `stale` | A revision-checked update saw a newer card revision. Nothing changes; the board reloads. Exit 1. |
| `unknown-project` | The project slug does not exist. Exit 1. |
| `project-exists` | The slug has different fields, or another project owns the GitHub repository. Exit 1. |
| `unknown-task` | No task matches the reference. Exit 1. |
| `agent-cannot-finish` | Agents cannot move cards to done or shipped. Exit 1. |
| `agent-cannot-archive` | Agents cannot archive or unarchive cards. Exit 1. |
| `agent-cannot-manage-projects` | Agents cannot add or archive projects. Exit 1. |
| `agent-cannot-edit` | Agents may append notes and edit priority, due, and dependencies, but cannot replace titles or notes. Exit 1. |
| `column-is-derived` | GitHub card columns cannot be moved by hand. Exit 1. |
| `github-owned-field` | GitHub owns the card title. Exit 1. |
| `after-not-done` | A dependency is unfinished when moving a local task to doing. Exit 1. |
| `after-self` | A task cannot depend on itself. Exit 1. |
| `after-cross-project` | Dependencies must belong to the same project. Exit 1. |
| `after-cycle` | The dependency would create a cycle. Exit 1. |
| `invalid-input` | Invalid arguments, date, priority, or plan JSON. Exit 2. |
| `store-too-new` | The store schema is newer than this binary supports. Exit 3. |
| `storage-failed` | Filesystem, SQLite, serialization failure, or exhausted task IDs/revisions. Read back before retrying a write. Exit 3. |

Success, including a no-op, exits 0. A successful JSON result has exactly
`ok`, `changed`, `data`, and `warnings`. Task creation returns
`data: {"task":…, "existing":bool}`. A no-op leaves revision and timestamps alone.

An add plan returns that success envelope with an ordered `data` array of
`{"status":"created","task":…}`, `{"status":"existing","task":…}`, or
`{"status":"failed","refusals":[…]}`. It exits 1 if any item failed validation;
valid items still commit together. `changed` is true if any item was created.
Malformed top-level JSON exits 2 without adding any tasks. A storage failure
rolls back the entire plan and exits 3 with a refusal envelope.

Help/version requests exit 0; with `--json`, their text is the success envelope's
`data` string. `punch --help` prints help without opening a store.

Without a subcommand, an interactive terminal opens the board. With JSON,
bare `punch --json` returns the help envelope; `punch board --json` exits 2
because the board is interactive. Without a terminal, bare `punch` and
`punch board` exit 2 with `invalid-input` before opening a store.
