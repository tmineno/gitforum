# git-forum MVP Specification

## 1. Purpose

This document defines the target MVP for `git-forum`.

The MVP is a Git-native work protocol for human-agent coding. It should let a repository manage
two first-class work objects, `rfc` and `issue`, with structured discussion, evidence, links,
state transitions, local search, and a lightweight TUI.

The MVP is intentionally local-first. It is for validating the model and the user experience, not
for reproducing a hosted issue tracker.

## 2. Goals

The MVP must satisfy these requirements.

1. A Git repository can create, display, and update `rfc` and `issue` threads.
2. Discussion is stored as typed nodes rather than a plain comment stream.
3. All changes are stored as append-only events in Git.
4. State transitions are validated by policy.
5. Threads can attach evidence and thread-to-thread links.
6. Issues can bind to Git branches.
7. Local search and list views are fast enough for everyday use.
8. Humans and AI actors can use the same CLI surface.
9. A lightweight TUI exists for list/detail browsing and basic creation flows.

## 2.1 Implementation constraints

- The main implementation language is Rust.
- The project must build and test on the stable Rust toolchain.
- Distribution should be a single `git-forum` binary.
- Git integration may use subprocesses or a library, but the semantics in this spec are the source
  of truth.

## 3. Non-goals

The MVP explicitly does not include:

- a separate `decision` thread kind
- mandatory AI provenance tracking
- AI-only high-level command sets
- a Web UI
- a central server
- real-time collaboration
- advanced access control
- automatic patch application
- large PM-style workflow systems
- embedding-based recommendation systems

## 4. Core model

### 4.1 Thread kinds

The target MVP has exactly two thread kinds:

- `rfc`
- `issue`

### 4.2 RFC-first workflow

- New projects, features, and design changes start as `rfc`.
- Implementation work happens in linked `issue` threads.
- An accepted RFC, together with its latest summary, acts as the decision record.
- There is no separate first-class `decision` object in the target model.

### 4.3 Event

An event is an immutable change recorded in a thread history.

### 4.4 Node

A node is a typed contribution inside a thread.

### 4.5 Evidence

Evidence is a reference to a commit, file, test, benchmark, document, thread, or external source.

### 4.6 Actor

An actor is a participant, human or AI. MVP distinguishes actor identity, but does not require a
separate provenance object for AI participation.

### 4.7 Approval

An approval is a recorded human sign-off attached to a state transition event.

## 5. State machines

### 5.1 Issue

```text
open -> closed
closed -> open
```

### 5.2 RFC

```text
draft -> proposed
draft -> rejected
proposed -> under-review
proposed -> draft
under-review -> accepted
under-review -> rejected
under-review -> draft
```

State is derived from event replay. Mutable thread state must not be the sole source of truth.

## 6. Data model

### 6.1 Thread

Required fields:

- `id`
- `kind`
- `title`
- `status`
- `created_at`
- `created_by`

Optional fields:

- `body`
- `scope.branch`
- `links[]`

`status` is materialized for display. The authoritative history is the event stream.

### 6.2 Event

Required fields:

- `thread_id`
- `event_type`
- `created_at`
- `actor`
- `parents[]`

Conditionally required:

- `kind` and `title` on `create`
- `node_type` and `body` on `say`
- `body` on `edit`
- `target_node_id` on `edit`, `retract`, `resolve`, and `reopen`
- `new_state` on `state`
- `approvals[]` when required by policy

Allowed `event_type` values:

- `create`
- `say`
- `edit`
- `retract`
- `resolve`
- `reopen`
- `link`
- `state`
- `scope`
- `verify`
- `merge`

### 6.3 Node types

The target workflow uses these node types:

- `claim`
- `question`
- `objection`
- `alternative`
- `evidence`
- `summary`
- `action`
- `risk`
- `assumption`
- `review`

`review` is a holistic analysis of the entire thread, distinct from `claim` (single assertion) and
`summary` (consensus digest). Reviews are informational and typically not resolvable.

`objection` and `action` are open when created. `resolve` closes them. `reopen` reopens them.
Retracted nodes remain in history but no longer count as open.

### 6.4 Evidence

Required fields:

- `evidence_id`
- `kind`
- `ref`

Allowed evidence kinds:

- `commit`
- `file`
- `hunk`
- `test`
- `benchmark`
- `doc`
- `thread`
- `external`

### 6.5 Approval

Required fields:

- `actor_id`
- `approved_at`
- `mechanism`

The only required mechanism in MVP is `recorded`. Cryptographic signing is a future extension.

## 7. Storage layout

### 7.1 Git refs

Authoritative data lives under:

- `refs/forum/threads/<THREAD_ID>`
- `refs/forum/index/<THREAD_ID>`

`refs/forum/index/*` is a rebuildable materialized snapshot.

### 7.2 Working tree files

Shared repository files:

```text
.forum/
  policy.toml
  actors.toml
  templates/
    issue.md
    rfc.md
```

### 7.3 Local-only files

```text
.git/forum/
  index.sqlite
  local.toml
  logs/
```

`local.toml` is for local-only settings.

## 8. Policy

The MVP uses `.forum/policy.toml`.

Policy is responsible for:

- which node types a role may emit
- which state transitions a role may perform
- which guards apply to which transition
- which transitions require human approval

Example:

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

Minimum required guards:

- `no_open_objections`
- `no_open_actions`
- `at_least_one_summary`
- `one_human_approval`

## 9. CLI surface

The target MVP CLI surface is:

### Repository setup

```bash
git forum init
git forum doctor
git forum reindex
```

### Thread creation

```bash
git forum issue new <title> [--body <TEXT> | --body-file <PATH>] [--branch <BRANCH>]
git forum rfc new <title> [--body <TEXT> | --body-file <PATH>]
```

### Listing and display

```bash
git forum --help-llm
git forum ls [--branch <BRANCH>]
git forum issue ls [--branch <BRANCH>]
git forum rfc ls
git forum show <THREAD_ID>
git forum node show <NODE_ID>
git forum search <QUERY>
```

### Discussion

Primitive:

```bash
git forum say <THREAD_ID> --type <NODE_TYPE> --body <TEXT>
```

First-batch shorthand commands:

```bash
git forum claim <THREAD_ID> <TEXT>
git forum question <THREAD_ID> <TEXT>
git forum objection <THREAD_ID> <TEXT>
git forum summary <THREAD_ID> <TEXT>
git forum action <THREAD_ID> <TEXT>
git forum risk <THREAD_ID> <TEXT>
```

### Node lifecycle

```bash
git forum revise <THREAD_ID> <NODE_ID> --body <TEXT>
git forum retract <THREAD_ID> <NODE_ID>
git forum resolve <THREAD_ID> <NODE_ID>
git forum reopen <THREAD_ID> <NODE_ID>
```

### State changes

```bash
git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]...
git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]... [--resolve-open-actions]
git forum state bulk --to <NEW_STATE> [<THREAD_ID>...] [--branch <BRANCH>] [--kind <KIND>] [--status <STATUS>] [--sign <ACTOR_ID>]... [--resolve-open-actions] [--dry-run]
```

### Evidence and links

```bash
git forum evidence add <THREAD_ID> --kind <KIND> --ref <REF>
git forum link <FROM> <TO> --rel <REL>
git forum branch bind <THREAD_ID> <BRANCH>
git forum branch clear <THREAD_ID>
```

### Verification and policy

```bash
git forum verify <THREAD_ID>
git forum policy lint
git forum policy check <THREAD_ID> --transition <TRANSITION>
```

### TUI

```bash
git forum tui
git forum tui <THREAD_ID>
```

## 10. Command requirements

### 10.1 `issue new` and `rfc new`

- create a `create` event
- accept `--body` or `--body-file`
- allocate a display ID
- assign an initial state
- update the thread ref

Initial states:

- issue: `open`
- rfc: `draft`

### 10.2 `say` and shorthand commands

- append a `say` event
- validate the node type against policy if enforced
- shorthand commands are aliases for `say --type ...`

### 10.3 `revise`, `retract`, `resolve`, `reopen`

- `revise` appends an `edit` event
- `retract` appends a `retract` event
- `resolve` and `reopen` operate primarily on `objection` and `action`
- node commands accept full canonical IDs and unique prefixes of at least 8 characters

### 10.4 `show`

At minimum, show:

1. title / body / kind / state
2. branch scope
3. open objections
4. open actions
5. latest summary
6. evidence
7. links
8. timeline

### 10.5 `state`

- validate the transition
- evaluate guards
- attach approvals from `--sign`
- append a `state` event
- `Issue open -> closed` must fail when open actions remain and policy requires `no_open_actions`
- `--resolve-open-actions` is an explicit escape hatch

### 10.6 `state bulk`

- evaluate each target independently
- allow target selection by explicit IDs or `--branch` / `--kind` / `--status`
- default to partial apply
- exit non-zero if any target fails
- `--dry-run` reports outcomes without writing events

### 10.7 `evidence add` and `link`

- `evidence add` appends an evidence-bearing `link` event
- `--kind commit --ref <REV>` resolves `<REV>` to a canonical commit OID
- `link` records thread-to-thread relations
- common relation names are `implements`, `relates-to`, `depends-on`, and `blocks`

### 10.8 `verify`

MVP verification checks:

- guard violations
- missing summary before RFC acceptance
- unresolved objections before RFC acceptance
- unresolved actions before issue close

## 11. TUI requirements

The MVP TUI is read-first, with minimum creation flows.

Required capabilities:

1. thread list
2. kind filter
3. thread detail
4. node detail
5. refresh / reindex-backed load
6. thread create
7. node create
8. node resolve / reopen / retract
9. thread link create
10. basic mouse support for selection, scrolling, and submit clicks

Policy-sensitive operations may remain CLI-first.

## 12. ID scheme

### 12.1 Thread display IDs

- issue: `ISSUE-0001`
- rfc: `RFC-0001`

### 12.2 Canonical event IDs

The canonical ID of an event is the Git commit OID that stores that event.

### 12.3 Canonical node IDs

The canonical ID of a node is the Git commit OID of the `say` event that introduced that node.

### 12.4 Short-ID resolution

- full canonical IDs must always work
- exact match wins first
- otherwise a unique prefix of at least 8 characters is accepted
- `node show` resolves globally
- thread-scoped node commands resolve within the specified thread

## 13. Semantic merge

The MVP should support minimal semantic merge.

Auto-merge cases:

- concurrent addition of new `say` events
- concurrent addition of evidence
- concurrent addition of summaries

Conflict cases:

- concurrent terminal state changes on the same thread
- concurrent `resolve` / `reopen` on the same open item

Conflicts should surface as unresolved merge state in `show`.

## 14. Search

MVP search may remain lexical.

Minimum search surface:

- thread title
- thread body
- current node body
- thread / node ID
- kind
- state
- branch, when indexed

Results may stay thread-oriented, but node hits must indicate which node matched.

## 15. Import / export

MVP import/export is minimal.

Import:

- GitHub issue -> `issue`
- markdown RFC -> `rfc` with manual cleanup

Export:

- `issue` -> issue-like markdown or tracker-friendly format
- `rfc` -> markdown

## 16. Error handling

The CLI should return:

- a specific failure reason
- the violated guard, policy rule, or state-machine rule
- a hint about the next valid move when helpful

Examples:

- `transition draft->under-review is not valid for rfc; valid transitions from 'draft': [proposed, rejected]`
- `FAIL [no_open_objections] unresolved objections remain`

## 17. Testing strategy

The MVP test environment is local-first and service-free.

Required layers:

1. unit tests for replay, state machine, policy, guard, merge, index, and search
2. integration tests using temporary Git repositories
3. TUI tests using a non-interactive backend

Requirements:

- tests must not depend on global Git config
- tests must not require the network
- clock and ID generation must be replaceable
- stable outputs such as `show`, `verify`, export, and TUI rendering may use snapshots

CI baseline:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

## 18. Acceptance criteria

The MVP is complete when:

1. `git forum init` works in an empty Git repository
2. `issue` and `rfc` can be created
3. typed discussion can be added
4. evidence and links can be attached
5. policy-driven state validation works
6. `show` displays open objections, open actions, latest summary, and timeline
7. `verify` evaluates the minimum required guards
8. `reindex` rebuilds the local index
9. the TUI supports list/detail/basic creation flows
10. minimal semantic merge is implemented
11. the project builds and tests on stable Rust

## 19. Recommended implementation order

1. repository init
2. thread create / load
3. event append / replay
4. `show`
5. `say` / `revise` / `resolve` / `state`
6. evidence and links
7. policy and verify
8. shorthand discussion commands
9. SQLite index and search
10. TUI
11. semantic merge

## 20. Open questions after MVP

- whether all node types deserve shorthand commands
- how much role enforcement should move from policy parsing to hard enforcement
- how rich the TUI editing surface should become
- how import/export should map RFC and issue metadata
- how much semantic merge UX should be exposed in the CLI
