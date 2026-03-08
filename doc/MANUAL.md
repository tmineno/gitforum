# Manual

This document describes how to use the current `git-forum` CLI.
It is intentionally based on the commands that are implemented today, not future spec ideas.

## Install

```bash
cargo install --path .
git-forum --help
```

If you only want to try it during development, this also works:

```bash
cargo run -- --help
```

To print this manual verbatim for an LLM or another tool:

```bash
git-forum --help-llm
```

## Conventions

- thread kinds: `issue`, `rfc`, `decision`
- thread IDs: `ISSUE-0001`, `RFC-0001`, `DEC-0001`
- node IDs: printed by `git forum say`; canonical IDs are Git commit OIDs of the `say` event
- CLI/TUI displays of node and event OIDs usually show the first 16 characters
- node IDs in CLI arguments:
  - full IDs always work
  - if there is no exact match, a unique prefix of at least 8 characters is accepted
  - `git forum node show` resolves prefixes globally
  - `revise`, `retract`, `resolve`, and `reopen` resolve prefixes inside the specified thread
- actor:
  - if `--as` is omitted, the current actor is inferred from Git config
  - you can override it explicitly, for example `--as human/alice`

## Repository setup

Initialize `git-forum` inside a repository before using it:

```bash
git forum init
git forum doctor
git forum reindex
```

- `init`: creates `.forum/` and `.git/forum/`
- `doctor`: checks policy, templates, local index, and ref namespace health
- `reindex`: rebuilds the local index from Git refs

## Create threads

### Issue

```bash
git forum issue new "Parser fails on nested blocks"
git forum issue new "Parser fails on nested blocks" --body "Repro in src/parser/tests.rs"
git forum issue new "Parser fails on nested blocks" --body -
git forum issue new "Parser fails on nested blocks" --body-file ./tmp/issue-body.md
git forum issue new "Parser fails on nested blocks" --branch feat/parser-rewrite
git forum issue new "Parser fails on nested blocks" \
  --link-to RFC-0001 --rel implements
```

### RFC

```bash
git forum rfc new "Switch solver backend to trait objects"
git forum rfc new "Switch solver backend to trait objects" \
  --body "Needed to make plugin ABI stability explicit."
git forum rfc new "Switch solver backend to trait objects" --body -
```

### Decision

```bash
git forum decision new "Adopt trait backend for v2"
```

`--body -` reads the initial body from standard input, so you can avoid creating a temporary file.
`--branch <BRANCH>` binds the new thread to an existing Git branch.
`--link-to <THREAD_ID> --rel <REL>` creates the thread and immediately records one or more links from
the new thread to existing threads.

### List by kind

```bash
git forum issue ls
git forum rfc ls
git forum decision ls
```

## List and inspect threads

```bash
git forum ls
git forum show RFC-0001
```

`git forum show <THREAD_ID>` shows:

- title
- branch
- body
- kind
- status
- created_at
- created_by
- open objections
- open actions
- latest summary
- timeline

The timeline is displayed in `date ID author type body` order.

If the thread has evidence, links, or AI runs attached, they appear between the summary and the timeline:

```text
evidence: 1
  - a1b2c3d4  benchmark  bench/result.csv

links: 1
  - ISSUE-0001  implements

runs: 1
  - RUN-0001
```

## Search

```bash
git forum search migration
git forum search objection
git forum search RFC-0001
```

`git forum search <QUERY>` searches the local index across:

- thread title
- thread body
- thread kind and state
- thread ID
- current node body
- current node type and node ID

Results are still grouped by thread. If the match came from a current node, the output includes
the matching node under the thread row.

## Add discussion nodes

### Add a node

```bash
git forum say RFC-0001 --type claim --body "Needed for compatibility."
git forum say RFC-0001 --type question --body "What is the migration plan?"
git forum say RFC-0001 --type objection --body "Benchmarks are missing."
git forum say RFC-0001 --type summary --body "Current consensus is to keep both backends."
```

Valid node types:

- `claim`
- `question`
- `objection`
- `alternative`
- `evidence`
- `summary`
- `decision`
- `action`
- `risk`
- `assumption`

`summary` is not just another comment. In the default workflow it is the human-readable statement of
what the thread currently concludes, what objections were addressed, and what decision is ready to
be accepted. The default policy therefore requires at least one `summary` before an RFC can move to
`accepted`.

On success, the command prints the node ID.

```text
Added question 6f1d2c3b4a5e67890123456789abcdef01234567
```

### Revise a node

```bash
git forum revise RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567 \
  --body "What is the migration and rollback plan?"
git forum revise RFC-0001 6f1d2c3b \
  --body "What is the migration and rollback plan?"
```

### Retract / resolve / reopen a node

```bash
git forum retract RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum reopen RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b
```

- `resolve` / `reopen` are mainly for `objection` and `action`
- `retract` keeps history while marking the node inactive

## Inspect a single node

Use this when you want to inspect one node directly instead of reading the whole thread:

```bash
git forum node show 6f1d2c3b4a5e67890123456789abcdef01234567
git forum node show 6f1d2c3b
```

`git forum node show <NODE_ID>` shows:

- node ID
- the thread it belongs to
- kind
- current state: `open`, `resolved`, `retracted`
- created_at
- actor
- current body
- thread links, if the parent thread is linked to other threads
- the history related to that node

If a prefix is ambiguous, the command fails and prints candidate full IDs.

## Evidence and links

### Add evidence to a thread

```bash
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum evidence add RFC-0001 --kind commit --ref HEAD~1
git forum evidence add RFC-0001 --kind commit --ref abc123def456
git forum evidence add RFC-0001 --kind file --ref src/lib.rs
```

Valid evidence kinds: `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`, `thread`, `external`.

For `--kind commit`, `--ref` may be a full SHA, short SHA, branch, tag, or other Git revision
expression. `git-forum` resolves it to a commit and stores the canonical commit OID. If the revision
does not resolve to a commit object, the command fails.

On success, the command prints the first 8 characters of the evidence ID (the Git commit SHA of the Link event):

```text
Evidence added (a1b2c3d4)
```

### Link two threads

```bash
git forum link ISSUE-0001 RFC-0001 --rel implements
git forum link RFC-0001 DEC-0001 --rel relates-to
```

On success:

```text
ISSUE-0001 -> RFC-0001 (implements)
```

### Bind a thread to a Git branch

```bash
git forum branch bind ISSUE-0001 feat/parser-rewrite
git forum branch clear ISSUE-0001
```

This updates the thread's `scope.branch`. It is most useful for issues that track implementation work
on a feature branch, but the command is available for any thread kind.

## AI runs

### Spawn a run

```bash
git forum run spawn RFC-0001 --as ai/reviewer
```

Creates a run record at `refs/forum/runs/RUN-NNNN` with status `running`, and writes a `Spawn` event into the thread's timeline. On success:

```text
Spawned RUN-0001
```

### List runs

```bash
git forum run ls
```

### Show a run

```bash
git forum run show RUN-0001
```

`git forum run show <RUN_LABEL>` shows:

- label and status
- thread it was spawned for
- actor
- started / ended timestamps
- model (if recorded)
- result and confidence (if recorded)

## TUI

```bash
git forum tui
git forum tui RFC-0001
```

Current controls:

- list view:
  - `j` / `k`: move between threads
  - `enter`: open thread detail
  - `c`: create a new thread
  - `f`: cycle kind filter
  - `r`: refresh from Git into the local index
  - `q`: quit
- thread detail view:
  - `j` / `k`: move between nodes in the thread
  - `up` / `down`: scroll the thread body and timeline pane
  - `enter`: open the selected node detail
  - `c`: create a new node in the current thread
  - `r`: refresh the thread from Git
  - `esc` / `q`: go back to the thread list
- node detail view:
  - `c`: create a new node in the parent thread
  - `x`: resolve the current node
  - `o`: reopen the current node if it is resolved or retracted
  - `R`: retract the current node
  - `j` / `k`: scroll
  - `r`: refresh the node from Git
  - `esc` / `q`: go back to the parent thread detail
- create thread / create node:
  - `tab`: move between fields
  - `up` / `down`: cycle kind, or move within the node type dropdown when that field is active
  - in create node, move to `body` and press `enter` to open the multiline body editor
  - in create node, move to `submit` and press `enter` to create the node
  - in the body editor, `enter` inserts a newline and `ctrl+s` returns to the form
  - `esc`: cancel

## Change thread state

```bash
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum state RFC-0001 accepted --sign human/alice
git forum state RFC-0001 accepted --sign human/alice --sign human/bob
```

- `--sign` is recorded as an approval on the event
- whether the transition succeeds depends on the state machine and policy guards
- for RFCs, `proposed` means the author is declaring the RFC review-ready
- for RFCs, `under-review` means active review is in progress

## Verify and inspect policy

```bash
git forum verify RFC-0001
git forum policy lint
git forum policy check RFC-0001 --transition under-review->accepted
```

- `verify`: checks whether the thread already satisfies guard conditions for its next forward transition
- `policy lint`: validates `.forum/policy.toml`
- `policy check`: dry-runs guard evaluation for a specific transition

### The policy file

The policy file lives at `.forum/policy.toml`.

It is created automatically by `git forum init`, and it controls two kinds of configuration:

- role declarations under `[roles.<name>]`
- transition guard rules under `[[guards]]`

A default file looks like this:

```toml
[roles.reviewer]
can_say = ["objection", "evidence", "summary", "risk"]
can_transition = ["under-review->changes-requested"]

[roles.maintainer]
can_say = ["claim", "decision", "summary"]
can_transition = ["draft->proposed", "proposed->under-review", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]
```

### What the fields mean

- `can_say`: node types that a role is allowed to emit
- `can_transition`: thread state transitions that a role is allowed to perform
- `on`: the transition that a guard block applies to, written as `from->to`
- `requires`: the list of guard rules that must pass for that transition

### Guard rules currently understood by the implementation

- `no_open_objections`
- `at_least_one_summary`
- `one_human_approval`

### What is enforced today

Current implementation status is narrower than the long-term spec:

- `git forum state ...` evaluates guard rules from `[[guards]]`
- `git forum verify` evaluates those same guard rules in read-only mode
- `git forum policy lint` currently performs structural validation, mainly checking that guard transitions use the `from->to` format

Role sections such as `can_say` and `can_transition` are parsed and preserved, but they are not yet fully enforced across all commands.

### Editing the policy file

The file is plain TOML, so the normal workflow is:

1. edit `.forum/policy.toml`
2. run `git forum policy lint`
3. run `git forum policy check ...` or `git forum verify ...`

Example:

```bash
git forum policy lint
git forum policy check RFC-0001 --transition under-review->accepted
git forum verify RFC-0001
```

### What `git forum verify` actually does

`git forum verify` is read-only. It does not change the thread state and it does not attach approvals.

At the moment, it only evaluates a small set of forward transitions:

- RFC in `under-review` is checked against `under-review -> accepted`
- Decision in `proposed` is checked against `proposed -> accepted`
- other kinds or states currently return `ok` because no verify target is defined

In practice, this means `verify` is most useful right before an acceptance-like transition. It answers: "If I tried to move this thread forward now, which guard checks would fail?"

### Typical output

If all configured guards pass:

```text
RFC-0001: ok
```

If one or more guards fail:

```text
FAIL [no_open_objections] unresolved objections remain
FAIL [at_least_one_summary] no summary node found
```

### `verify` vs `policy check`

Use `verify` when you want the tool to infer the next forward transition from the current thread state.

Use `policy check` when you want to ask about a specific transition explicitly:

```bash
git forum policy check RFC-0001 --transition under-review->accepted
git forum policy check DEC-0001 --transition proposed->accepted
```

## Typical workflow

```bash
git forum init
git forum rfc new "Switch solver backend to trait objects" \
  --body "Needed to make plugin ABI stability explicit."
git forum say RFC-0001 --type claim --body "Needed for compatibility."
git forum say RFC-0001 --type objection --body "Benchmarks are missing."
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum say RFC-0001 --type summary --body "Benchmarks added; objection addressed."
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum issue new "Implement trait backend" --link-to RFC-0001 --rel implements
git forum run spawn RFC-0001 --as ai/reviewer
git forum show RFC-0001
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum verify RFC-0001
git forum state RFC-0001 accepted --sign human/alice
```

## Current scope

This manual currently covers:

- init / doctor / reindex
- issue / rfc / decision create and list
- thread show (with evidence / links / runs sections)
- node show
- say / revise / retract / resolve / reopen
- state
- verify
- policy lint / check
- evidence add
- link
- run spawn / ls / show
- TUI

Still out of scope:

- import / export
