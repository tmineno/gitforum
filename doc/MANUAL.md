# Manual

## Quick Reference

```
git forum init                                     Initialize forum in repo
git forum new issue "Title" --body "..."           Create an issue
git forum ls --kind issue                          List issues
git forum show ISSUE-0001                          Show issue details
git forum claim ISSUE-0001 "Implemented X"       Add a claim node
git forum close ISSUE-0001                         Close an issue
git forum close ISSUE-0001 --comment "Done"        Close with summary
git forum pend ISSUE-0001                          Mark issue pending
git forum new rfc "Title" --body "..."             Create an RFC
git forum accept RFC-0001 --sign human/alice       Accept an RFC
git forum show RFC-0001 --what-next                Show valid next actions
git forum evidence add ISSUE-0001 --kind commit --ref HEAD  Add evidence
git forum status --all                             Check open items
git forum tui                                      Open interactive TUI
```

---

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

**Note:** `git forum --help` requires `git forum init` to have been run first.
`init` sets a local git alias (`alias.forum = !git-forum`) so that `--help` is
passed to the binary instead of triggering Git's man-page lookup.

To print this manual verbatim for an LLM or another tool:

```bash
git-forum --help-llm                   # full manual
git-forum claim --help-llm             # node type taxonomy
git-forum state --help-llm             # state transition map
git-forum evidence --help-llm          # evidence kinds reference
```

Per-subcommand `--help-llm` prints only the relevant reference section. Shorthand node commands
(`claim`, `question`, `objection`, `summary`, `action`, `risk`, `review`)
print the node type taxonomy. `state` (and shorthand commands like `close`, `accept`)
prints the state transition map. `evidence` prints evidence kinds.

## Conventions

- thread kinds: `issue`, `rfc`
- thread IDs: `ISSUE-0001`, `RFC-0001`
- node IDs: printed by shorthand node commands (e.g. `claim`, `question`); canonical IDs are Git commit OIDs of the say event
- CLI/TUI displays of node and event OIDs usually show the first 16 characters
- node IDs in CLI arguments:
  - full IDs always work
  - if there is no exact match, a unique prefix of at least 8 characters is accepted
  - `git forum node show` resolves prefixes globally
  - `revise node`, `retract`, `resolve`, and `reopen` resolve prefixes inside the specified thread
- actor:
  - resolution order: `--as` flag → `GIT_FORUM_ACTOR` env var → Git config `user.name`
  - `--as human/alice` or `--as ai/reviewer` overrides everything
  - `GIT_FORUM_ACTOR=ai/reviewer` persists across commands without repeating `--as`
  - if neither is set, the actor is inferred from Git config as `human/<slug>`

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
git forum new rfc "Switch solver backend to trait objects"
git forum new rfc "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum new rfc "Switch solver backend to trait objects" --body -
git forum new rfc "Switch solver backend to trait objects" --body-file ./tmp/rfc.md
```

### Issue

Use issues for implementation work, especially when code or a branch is involved.

```bash
git forum new issue "Implement trait backend"
git forum new issue "Implement trait backend" --body "Initial implementation checklist"
git forum new issue "Implement trait backend" --body -
git forum new issue "Implement trait backend" --body-file ./tmp/issue.md
git forum new issue "Implement trait backend" --branch feat/trait-backend
git forum new issue "Implement trait backend" \
  --link-to RFC-0001 --rel implements
git forum new rfc "Error handling" \
  --claim "All errors should be typed" --action "Define error enum"
```

`--body -` reads the initial body from standard input, so you can avoid creating a temporary file.

### Inline nodes at creation

Thread creation accepts `--claim`, `--question`, `--objection`, `--action`, `--risk`, and
`--summary` flags to add nodes immediately after creating the thread. Each flag may be repeated:

```bash
git forum new rfc "Caching layer" --body "Goal and constraints." \
  --claim "LRU eviction with 10-min TTL" \
  --action "Benchmark cache hit ratio" \
  --risk "Memory pressure under load"
```

This is equivalent to running `new rfc` followed by separate `claim`, `action`, and `risk` commands.
`--branch <BRANCH>` binds the new thread to an existing Git branch.
`--link-to <THREAD_ID> --rel <REL>` creates the thread and immediately records one or more links
from the new thread to existing threads.

### Create from a commit

```bash
git forum new issue --from-commit HEAD
git forum new issue --from-commit abc123 --link-to RFC-0001 --rel implements
```

`--from-commit <REV>` uses the commit subject as the title, the commit body as the thread body,
and automatically adds the commit as evidence. An explicit title argument overrides the subject.

### Create from another thread

```bash
git forum new issue --from-thread RFC-0001
git forum new rfc --from-thread RFC-0003
git forum new issue --from-thread RFC-0001 "Custom title"
```

`--from-thread <THREAD_ID>` copies the title (prefixed with `v2: `) and body from the source
thread, creates bidirectional `supersedes` / `superseded-by` links, and auto-deprecates the source
thread if it is an RFC. An explicit title argument overrides the default title. This is useful for
creating successor RFCs from deprecated ones or implementation issues from accepted RFCs.

### List by kind

```bash
git forum ls --kind issue
git forum ls --kind issue --branch feat/trait-backend
git forum ls --kind rfc
```

The old forms `git forum issue ls` and `git forum rfc ls` remain as hidden aliases for backward
compatibility.

## List and inspect threads

```bash
git forum ls
git forum ls --branch feat/trait-backend
git forum show RFC-0001
git forum show RFC-0001 --what-next
```

`git forum ls` shows `ID`, `KIND`, `STATUS`, `BRANCH`, `CREATED`, `UPDATED`, and `TITLE`.
`--kind rfc` or `--kind issue` filters by thread kind.
`--branch <BRANCH>` filters the listing to threads currently bound to that branch.

`git forum show <THREAD_ID>` shows:

- title
- branch
- body
- kind
- status
- created_at
- created_by
- body revisions count (if body has been revised)
- incorporated nodes (if any)
- open objections
- open actions
- latest summary
- evidence section
- links section
- conversations (reply chains grouped by root node)
- timeline

`git forum show <THREAD_ID> --what-next` shows valid next actions:

```text
RFC-0001 (under-review)

valid transitions: accepted, rejected, draft

guard check (under-review -> accepted):
  [FAIL] no_open_objections -- 1 open objection(s)

open objections: 1
open actions:    0
nodes:           6
evidence:        1
links:           0
has summary:     yes
```

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

Each node type has a dedicated shorthand command.
All node commands accept a positional body argument, `--body-file`, and `--as`. Pass `"-"` as the
positional body to read from stdin.

```bash
git forum claim RFC-0001 "Need a stable plugin-facing boundary."
git forum question RFC-0001 "What compatibility risks remain?"
git forum objection RFC-0001 "Benchmarks are missing."
git forum summary RFC-0001 "Direction is sound, but migration evidence is missing."
git forum action ISSUE-0001 "Add branch-local benchmark fixture."
git forum risk ISSUE-0001 "Parser behavior may diverge under edge inputs."
git forum review RFC-0001 "Overall analysis of the RFC."
git forum objection RFC-0001 --body-file ./tmp/detailed-objection.md
git forum claim RFC-0001 --body -
```

Supported shorthand commands:

- `git forum claim`
- `git forum question`
- `git forum objection`
- `git forum summary`
- `git forum action`
- `git forum risk`
- `git forum review`

Valid node types used in the preferred workflow:

- `claim`
- `question`
- `objection`
- `evidence`
- `summary`
- `action`
- `risk`
- `review`

`review` is a holistic analysis of the entire thread, distinct from `claim` (single assertion) and
`summary` (consensus digest). Reviews are informational and typically not resolvable.

`summary` is not just another comment. In the default workflow it is the human-readable statement of
what the thread currently concludes, what objections were addressed, and what is ready to move
forward. The default policy therefore requires at least one `summary` before an RFC can move to
`accepted`.

On success, the command prints the node ID to stdout and a next-actions hint to stderr:

```text
Added question 6f1d2c3b4a5e67890123456789abcdef01234567
  next: proposed, rejected
  open: 1 open objection(s)
```

The hint shows valid state transitions and open items. Suppress with `2>/dev/null`.

### Revise a node

```bash
git forum revise node RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567 \
  --body "What is the migration and rollback plan?"
git forum revise node RFC-0001 6f1d2c3b \
  --body "What is the migration and rollback plan?"
```

Use `revise node` to update an existing node when the intent is the same but the content needs
correction. For example, revise a summary to incorporate new objections rather than adding a
second summary node. The revision history is preserved and visible in `git forum node show`.

### Retract / resolve / reopen a node

```bash
git forum retract RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum reopen RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b
```

- `resolve` / `reopen` are mainly for `objection` and `action`
- `retract` keeps history while marking the node inactive

### Reply to a node

Use `--reply-to` to link a node as a response to an existing node:

```bash
git forum claim RFC-0001 "Tests added, benchmark in bench/result.csv" \
  --reply-to <OBJECTION_NODE_ID>
git forum question RFC-0001 "Can you clarify X?" --reply-to <CLAIM_NODE_ID>
```

`--reply-to` is accepted on all shorthand node commands. Reply chains of arbitrary depth are
supported. `git forum show` groups reply chains into conversations for readability.

### Revise thread body

```bash
git forum revise body RFC-0001 --body "Updated body text"
git forum revise body RFC-0001 --body-file ./tmp/body.md
git forum revise body RFC-0001 --body -
```

`--incorporates` marks referenced nodes as incorporated into this revision:

```bash
git forum revise body RFC-0001 --body "Revised body" \
  --incorporates 6f1d2c3b --incorporates a1b2c3d4
```

Incorporated nodes appear as `incorporated` status in show output, distinct from `resolved` and
`retracted`. They represent content that has been folded into the current body.

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
- current state: `open`, `resolved`, `retracted`, `incorporated`
- in reply to (if this node is a reply)
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
git forum evidence add ISSUE-0001 --kind commit --ref abc123 def456 789012
git forum evidence add ISSUE-0001 --kind file --ref src/lib.rs
git forum evidence add ISSUE-0001 --kind test --ref tests/backend_trait.rs
```

`--ref` accepts multiple values in a single command. Each ref creates its own evidence event.

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

### Colors

The TUI uses color to distinguish kinds, statuses, and node types:

- **Thread kind**: cyan = rfc, yellow = issue
- **Thread status**: green = open/draft, yellow = pending/proposed/under-review,
  magenta = accepted/closed, red = rejected, gray = deprecated
- **Node type**: red = objection/risk, yellow = question, green = summary, cyan = action,
  blue = review
- **Node status**: green = open, gray = resolved/retracted/incorporated

Resolved, retracted, and incorporated node rows are dimmed.

Current controls:

- list view:
  - `j` / `k`: move between threads
  - single click on a thread row: select it
  - double click on a thread row: open thread detail
  - `enter`: open thread detail
  - `c`: create a new thread
  - `f`: cycle kind filter
  - `r`: refresh from Git into the local index
  - click a column header: sort by that column (click again to toggle ascending/descending)
  - mouse wheel: move through the list
  - `q`: quit
- thread detail view:
  - `j` / `k`: move between nodes in the thread
  - `up` / `down`: scroll the thread body and timeline pane
  - single click on a node row: select it
  - double click on a node row: open node detail
  - mouse wheel over the left pane: scroll the thread body and timeline pane
  - `enter`: open the selected node detail
  - `c`: create a new node in the current thread
  - `l`: create a thread link from the current thread
  - `m`: toggle markdown rendering for the thread body pane
  - `S`: enter select mode for pane-scoped text selection (copy with mouse)
  - `r`: refresh the thread from Git
  - `esc` / `q`: go back to the thread list
- node detail view:
  - `c`: create a new node in the parent thread
  - `l`: create a thread link on the parent thread
  - `x`: resolve the current node
  - `o`: reopen the current node if it is resolved or retracted
  - `R`: retract the current node
  - `m`: toggle markdown rendering for the node body
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

## Status

```bash
git forum status RFC-0001
git forum status --all
```

`git forum status <THREAD_ID>` shows unresolved items grouped by type: open objections, open
actions, and open questions.

`git forum status --all` shows unresolved items across all open threads, omitting threads with no
open items.

## Change thread state

### Shorthand commands

All state shorthands are now top-level (verb-first):

```bash
git forum close ISSUE-0001
git forum close ISSUE-0001 --comment "Fixed in abc123"
git forum close ISSUE-0001 --link-to RFC-0001 --rel implements
git forum close ISSUE-0001 --resolve-open-actions
git forum pend ISSUE-0001                              # mark as pending
git forum pend ISSUE-0001 --comment "Waiting on review"
git forum reopen ISSUE-0001
git forum reject ISSUE-0001 --comment "Won't fix"
git forum propose RFC-0001
git forum accept RFC-0001 --sign human/alice
git forum deprecate RFC-0001 --comment "Superseded by RFC-0005"
git forum state RFC-0001 deprecated --link-to RFC-0005 --rel relates-to
```

Shorthand commands combine a state transition with optional `--comment` (adds a summary node before
transitioning), `--link-to` (creates links after transitioning), and `--sign` (records approvals).

Available shorthands:
- `close` — transition to `closed` (also accepts `--sign`, `--link-to`, `--rel`, `--resolve-open-actions`)
- `pend` — transition to `pending` (work-in-progress)
- `reopen` — transition to `open` (1 arg: thread state reopen; 2 args: node reopen)
- `reject` — transition to `rejected`
- `propose` — transition to `proposed`
- `accept` — transition to `accepted` (also accepts `--sign`, `--link-to`, `--rel`)
- `deprecate` — transition to `deprecated` (from `accepted` or `rejected`)

The old kind-prefixed forms (`git forum issue close`, `git forum rfc accept`, etc.) remain as hidden
aliases for backward compatibility.

### Generic state command

```bash
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum state RFC-0001 accepted --sign human/alice
git forum state ISSUE-0001 closed --resolve-open-actions
git forum state ISSUE-0001 closed --comment "Done" --link-to RFC-0001 --rel implements
git forum state bulk --to closed --branch v0.1.0
git forum state bulk --to closed ISSUE-0001 ISSUE-0002 --dry-run
```

- `--sign` is recorded as an approval on the event
- `--comment` adds a summary node before the state transition
- `--link-to` and `--rel` create thread links after the state transition
- whether the transition succeeds depends on the state machine and policy guards
- for RFCs, `proposed` means the author is declaring the RFC review-ready
- for RFCs, `under-review` means active review is in progress
- an accepted RFC is the decision record; there is no separate decision workflow in the preferred model
- issues support `open`, `pending`, `closed`, and `rejected` states; `pending` is for
  work-in-progress or waiting, `rejected` is for invalid or won't-fix issues, `closed` means
  completed
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

- transition guard rules under `[[guards]]`

A default file looks like this:

```toml
[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]
```

### What the fields mean

- `on`: the transition that a guard block applies to, written as `from->to`
- `requires`: the list of guard rules that must pass for that transition

### Guard rules currently understood by the implementation

- `no_open_objections`
- `no_open_actions`
- `at_least_one_summary`
- `one_human_approval`
- `has_commit_evidence`

### What is enforced today

Current implementation status is narrower than the target spec:

- `git forum state ...` evaluates guard rules from `[[guards]]`
- `git forum verify` evaluates those same guard rules in read-only mode
- `git forum policy lint` currently performs structural validation, mainly checking that guard
  transitions use the `from->to` format


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
git forum new rfc "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum claim RFC-0001 "Needed for compatibility."
git forum question RFC-0001 "What is the migration plan?" --as ai/reviewer
git forum objection RFC-0001 "Benchmarks are missing."
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum summary RFC-0001 "Benchmarks added; objection addressed."
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum propose RFC-0001
git forum state RFC-0001 under-review
git forum verify RFC-0001
git forum accept RFC-0001 --sign human/alice
git forum new issue "Implement trait backend" --link-to RFC-0001 --rel implements
git forum branch bind ISSUE-0001 feat/trait-backend
git forum action ISSUE-0001 "Wire trait backend behind feature flag."
git forum evidence add ISSUE-0001 --kind test --ref tests/backend_trait.rs
git forum close ISSUE-0001
```

## AI-agent workflow pattern

A common pattern with coding agents (AI reviewer, AI implementer) uses the same CLI surface as
human participants. The typical flow is:

1. A human or agent opens an RFC and adds initial claims.
2. An AI reviewer posts objections and questions using `--as ai/reviewer`.
3. An implementer (human or agent) replies to each objection with evidence or claims.
4. A human resolves addressed objections, adds a summary, and signs the acceptance.

```bash
# 1. Human opens the RFC
git forum new rfc "Add caching layer" --body "Goal and constraints."

# 2. AI reviewer raises concerns
GIT_FORUM_ACTOR=ai/reviewer
git forum objection RFC-0001 "No eviction strategy described."
git forum question RFC-0001 "What is the expected cache hit ratio?"

# 3. Implementer responds to the objection
git forum claim RFC-0001 "LRU eviction with 10-minute TTL." \
  --reply-to <OBJECTION_NODE_ID>

# 4. Human resolves the objection, summarizes, and accepts
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum summary RFC-0001 "Caching with LRU eviction approved."
git forum propose RFC-0001
git forum state RFC-0001 under-review
git forum accept RFC-0001 --sign human/alice
```

## Linking implementation commits as evidence

After implementing work on a branch, link the commits back to the RFC or issue so that the
decision trail connects to the code:

```bash
# Link the commit that implements the feature
git forum evidence add ISSUE-0001 --kind commit --ref HEAD
git forum evidence add ISSUE-0001 --kind commit --ref abc123

# Link a test file as evidence
git forum evidence add ISSUE-0001 --kind test --ref tests/cache_test.rs

# Link back to the RFC that motivated this issue
git forum evidence add ISSUE-0001 --kind thread --ref RFC-0001
```

`--kind commit --ref` accepts any Git revision expression (SHA, branch, tag, `HEAD~1`). The
resolved commit OID is stored canonically.

## Concurrency

`git-forum` uses Git's atomic ref updates (compare-and-swap) to detect concurrent writes. Each
`write_event` call reads the current thread ref tip, creates a new commit, and atomically updates
the ref only if the tip has not changed since it was read.

If two writers attempt to update the same thread simultaneously, one will succeed and the other will
fail with a clear error:

```text
concurrent write conflict on refs/forum/threads/RFC-0001: expected <sha> but ref was updated by another writer. Retry your command.
```

**Recommended patterns for parallel agent workflows:**

- **Different threads**: fully safe in parallel. Each thread has its own ref.
- **Same thread**: serialize writes, or retry on conflict. Conflicts are rare for human workflows
  but more likely when multiple agents update the same thread simultaneously.
- **Create vs update**: thread creation uses `create_ref` which fails if the ref already exists,
  preventing duplicate thread IDs.

This is related to ISSUE-0006 (semantic merge for concurrent events), which would automatically
resolve non-conflicting concurrent writes to the same thread.

## Current scope

This manual currently covers:

- init / doctor / reindex
- thread create (`new issue`, `new rfc`) and list (`ls --kind`)
- thread show
- node show
- search
- node commands (claim, question, objection, etc.) with --reply-to
- revise body / revise node
- retract / resolve / reopen
- status
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
