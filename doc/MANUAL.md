# Manual

This manual describes the preferred `git-forum` workflow:

- start work with `rfc`
- implement with `issue`
- use typed discussion instead of plain comments
- let humans and agents use the same CLI surface

## Install

```bash
cargo install --path .
git-forum --help
```

If you only want to try it during development:

```bash
cargo run -- --help
```

To print this manual verbatim for an LLM or another tool:

```bash
git-forum --help-llm
```

## Conventions

- thread kinds: `issue`, `rfc`
- thread IDs: `ISSUE-0001`, `RFC-0001`
- node IDs: printed by `git forum say`; canonical IDs are Git commit OIDs of the `say` event
- CLI/TUI displays of node and event OIDs usually show the first 16 characters
- node IDs in CLI arguments:
  - full IDs always work
  - if there is no exact match, a unique prefix of at least 8 characters is accepted
  - `git forum node show` resolves prefixes globally
  - `revise`, `retract`, `resolve`, and `reopen` resolve prefixes inside the specified thread
- actor:
  - if `--as` is omitted, the current actor is inferred from Git config
  - you can override it explicitly, for example `--as human/alice` or `--as ai/reviewer`

## Preferred model

- `rfc` is the starting point for a project, feature, or design change
- `issue` is the implementation work item
- an accepted RFC plus its latest summary acts as the decision record
- agents are participants, not a separate control plane

In other words: do not start with a standalone `decision` object. Start with an RFC, then create
linked issues once the RFC is accepted.

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

### RFC

Use RFCs to frame work before implementation starts.

```bash
git forum rfc new "Switch solver backend to trait objects"
git forum rfc new "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum rfc new "Switch solver backend to trait objects" --body -
git forum rfc new "Switch solver backend to trait objects" --body-file ./tmp/rfc.md
```

### Issue

Use issues for implementation work, especially when code or a branch is involved.

```bash
git forum issue new "Implement trait backend"
git forum issue new "Implement trait backend" --body "Initial implementation checklist"
git forum issue new "Implement trait backend" --body -
git forum issue new "Implement trait backend" --body-file ./tmp/issue.md
git forum issue new "Implement trait backend" --branch feat/trait-backend
git forum issue new "Implement trait backend" \
  --link-to RFC-0001 --rel implements
```

`--body -` reads the initial body from standard input, so you can avoid creating a temporary file.
`--branch <BRANCH>` binds the new thread to an existing Git branch.
`--link-to <THREAD_ID> --rel <REL>` creates the thread and immediately records one or more links
from the new thread to existing threads.

### List by kind

```bash
git forum issue ls
git forum issue ls --branch feat/trait-backend
git forum rfc ls
```

## List and inspect threads

```bash
git forum ls
git forum ls --branch feat/trait-backend
git forum show RFC-0001
```

`git forum ls` and kind-specific `ls` commands show `ID`, `KIND`, `STATUS`, `BRANCH`, and `TITLE`.
`--branch <BRANCH>` filters the listing to threads currently bound to that branch.

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
- evidence section
- links section
- timeline

The timeline is displayed in `date node_id event_id author type body` order.

If the thread has evidence or links attached, they appear between the summary and the timeline:

```text
evidence: 1
  - a1b2c3d4  benchmark  bench/result.csv

links: 1
  - ISSUE-0001  implements
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

Results are grouped by thread. If the match came from a current node, the output includes the
matching node under the thread row.

## Add structured discussion

### Add a node

Use shorthand commands for common node types. `say --type` remains the primitive fallback.

```bash
git forum claim RFC-0001 "Need a stable plugin-facing boundary."
git forum question RFC-0001 "What compatibility risks remain?"
git forum objection RFC-0001 "Benchmarks are missing."
git forum summary RFC-0001 "Direction is sound, but migration evidence is missing."
git forum action ISSUE-0001 "Add branch-local benchmark fixture."
git forum risk ISSUE-0001 "Parser behavior may diverge under edge inputs."
```

Supported shorthand commands:

- `git forum claim`
- `git forum question`
- `git forum objection`
- `git forum summary`
- `git forum action`
- `git forum risk`

Valid node types used in the preferred workflow:

- `claim`
- `question`
- `objection`
- `alternative`
- `evidence`
- `summary`
- `action`
- `risk`
- `assumption`

`summary` is not just another comment. In the default workflow it is the human-readable statement of
what the thread currently concludes, what objections were addressed, and what is ready to move
forward. The default policy therefore requires at least one `summary` before an RFC can move to
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
git forum evidence add ISSUE-0001 --kind commit --ref HEAD~1
git forum evidence add ISSUE-0001 --kind commit --ref abc123def456
git forum evidence add ISSUE-0001 --kind file --ref src/lib.rs
git forum evidence add ISSUE-0001 --kind test --ref tests/backend_trait.rs
```

Valid evidence kinds: `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`, `thread`, `external`.

For `--kind commit`, `--ref` may be a full SHA, short SHA, branch, tag, or other Git revision
expression. `git-forum` resolves it to a commit and stores the canonical commit OID. If the
revision does not resolve to a commit object, the command fails.

On success, the command prints the first 8 characters of the evidence ID, which is the Git commit
SHA of the `link` event:

```text
Evidence added (a1b2c3d4)
```

### Link two threads

```bash
git forum link ISSUE-0001 RFC-0001 --rel implements
git forum link ISSUE-0002 ISSUE-0001 --rel depends-on
git forum link ISSUE-0003 ISSUE-0002 --rel blocks
git forum link RFC-0002 RFC-0001 --rel relates-to
```

On success:

```text
ISSUE-0001 -> RFC-0001 (implements)
```

`--rel` is currently free-form. Common values are `implements`, `relates-to`, `depends-on`, and
`blocks`.

### Bind a thread to a Git branch

```bash
git forum branch bind ISSUE-0001 feat/parser-rewrite
git forum branch clear ISSUE-0001
```

This updates the thread's `scope.branch`. It is most useful for issues that track implementation
work on a feature branch, but the command is available for any thread kind.

## TUI

```bash
git forum tui
git forum tui RFC-0001
```

Current controls:

- list view:
  - `j` / `k`: move between threads
  - left click on a thread row: open thread detail
  - `enter`: open thread detail
  - `c`: create a new thread
  - `f`: cycle kind filter
  - `r`: refresh from Git into the local index
  - mouse wheel: move through the list
  - `q`: quit
- thread detail view:
  - `j` / `k`: move between nodes in the thread
  - `up` / `down`: scroll the thread body and timeline pane
  - left click on a node row: open node detail
  - mouse wheel over the left pane: scroll the thread body and timeline pane
  - `enter`: open the selected node detail
  - `c`: create a new node in the current thread
  - `l`: create a thread link from the current thread
  - `r`: refresh the thread from Git
  - `esc` / `q`: go back to the thread list
- node detail view:
  - `c`: create a new node in the parent thread
  - `l`: create a thread link on the parent thread
  - `x`: resolve the current node
  - `o`: reopen the current node if it is resolved or retracted
  - `R`: retract the current node
  - `j` / `k`: scroll
  - mouse wheel: scroll the node detail text
  - `r`: refresh the node from Git
  - `esc` / `q`: go back to the parent thread detail
- create thread / create node / create link:
  - `tab`: move between fields
  - `up` / `down`: cycle kind in create thread, or move within the node type dropdown in create node
  - in create thread, move to `body` and press `enter` to open the multiline body editor
  - in create thread, move to `submit` and press `enter` to create the thread
  - in create node, move to `body` and press `enter` to open the multiline body editor
  - in create node, move to `submit` and press `enter` to create the node
  - in create link, choose a relation, choose a target kind, then select a matching thread when the
    target kind is auto-resolvable
  - the TUI link form currently offers the common relations `implements`, `relates-to`,
    `depends-on`, and `blocks`
  - in create link, choose `manual` target kind to type a thread ID directly
  - in create link, move to `submit` and press `enter` to create the link
  - clicking the `submit` row also submits the current form
  - in the body editor, `enter` inserts a newline and `ctrl+s` returns to the form
  - `esc`: cancel

## Change thread state

```bash
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum state RFC-0001 accepted --sign human/alice
git forum state ISSUE-0001 closed --resolve-open-actions
git forum state bulk --to closed --branch v0.1.0
git forum state bulk --to closed ISSUE-0001 ISSUE-0002 --dry-run
```

- `--sign` is recorded as an approval on the event
- whether the transition succeeds depends on the state machine and policy guards
- for RFCs, `proposed` means the author is declaring the RFC review-ready
- for RFCs, `under-review` means active review is in progress
- an accepted RFC is the decision record; there is no separate decision workflow in the preferred model
- if policy requires `no_open_actions`, closing an issue with open `action` nodes fails
- `--resolve-open-actions` is an explicit escape hatch for issue close; it resolves open `action`
  nodes before writing the closing state event
- `state bulk` evaluates each target independently, applies successful transitions, reports
  failures inline, and exits non-zero if any target failed
- `state bulk --dry-run` reports what would succeed or fail without writing any events

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
can_say = ["question", "objection", "summary", "risk"]
can_transition = []

[roles.maintainer]
can_say = ["claim", "summary", "action"]
can_transition = ["draft->proposed", "proposed->under-review", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]
```

### What the fields mean

- `can_say`: node types that a role is allowed to emit
- `can_transition`: thread state transitions that a role is allowed to perform
- `on`: the transition that a guard block applies to, written as `from->to`
- `requires`: the list of guard rules that must pass for that transition

### Guard rules currently understood by the implementation

- `no_open_objections`
- `no_open_actions`
- `at_least_one_summary`
- `one_human_approval`

### What is enforced today

Current implementation status is narrower than the target spec:

- `git forum state ...` evaluates guard rules from `[[guards]]`
- `git forum verify` evaluates those same guard rules in read-only mode
- `git forum policy lint` currently performs structural validation, mainly checking that guard
  transitions use the `from->to` format

Role sections such as `can_say` and `can_transition` are parsed and preserved, but they are not yet
fully enforced across all commands.

### What `git forum verify` actually does

`git forum verify` is read-only. It does not change the thread state and it does not attach approvals.

At the moment, it evaluates these forward transitions:

- RFC in `under-review` against `under-review -> accepted`
- Issue in `open` against `open -> closed`
- other kinds or states currently return `ok` because no verify target is defined

In practice, this means `verify` is most useful right before an acceptance-like transition. It answers:
"If I tried to move this thread forward now, which guard checks would fail?"

## Typical workflow

```bash
git forum init
git forum rfc new "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum say RFC-0001 --type claim --body "Needed for compatibility."
git forum say RFC-0001 --type question --body "What is the migration plan?" --as ai/reviewer
git forum say RFC-0001 --type objection --body "Benchmarks are missing."
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum say RFC-0001 --type summary --body "Benchmarks added; objection addressed."
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum verify RFC-0001
git forum state RFC-0001 accepted --sign human/alice
git forum issue new "Implement trait backend" --link-to RFC-0001 --rel implements
git forum branch bind ISSUE-0001 feat/trait-backend
git forum say ISSUE-0001 --type action --body "Wire trait backend behind feature flag."
git forum evidence add ISSUE-0001 --kind test --ref tests/backend_trait.rs
git forum state ISSUE-0001 closed
```

## Current scope

This manual currently covers:

- init / doctor / reindex
- issue / rfc create and list
- thread show
- node show
- search
- say / revise / retract / resolve / reopen
- state
- verify
- policy lint / check
- evidence add
- link
- branch bind / clear
- TUI

Still out of scope:

- import / export
- merge conflict resolution UX
