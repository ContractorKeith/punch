# GitHub fixtures

`recorded-issues.json` and `recorded-prs.json` are unmodified, read-only public
GitHub snapshots captured on 2026-11-01 with:

```sh
gh issue list --repo cli/cli --limit 1 --json number,title,state,assignees,url,updatedAt
gh pr list --repo cli/cli --limit 1 --json number,title,state,assignees,url,updatedAt,isDraft,reviewDecision,author
```

`derivation.json` contains controlled variants of those snapshots for every
section 5 rule, including closed and merged states. Its `columns` are explicit
expected results. `open.json` is a minimal pair for focused sync/claim tests.
Both include the adapter's `kind` discriminator; `closes` contains issue numbers
in the target repository after checking GraphQL `closingIssuesReferences`.

`gh` is a fake executable installed first on PATH in isolated test subprocesses.
It serves the recorded snapshots, controlled two-page GraphQL responses (with
same-number issues in different repositories), and fake assignment results.
Every test subprocess has its own temporary PUNCH_STATE_DIR. No fixture test
can reach a real GitHub repository.
