# git-forum Product Specification

Version 1.1 — 2026-03-17

## 1. Overview

`git-forum` is a Git-native structured discussion and work-tracking tool for human-agent software
development. It stores RFCs, issues, typed discussion, evidence, and state transitions as
append-only events inside a Git repository.

### 1.1 Design principles

1. **Git-native.** All authoritative data lives in Git objects and refs. No external database is
   required for correctness.
2. **Event-sourced.** Thread state is derived from replaying an immutable event stream. Events are
   Git commits; the commit graph is the audit trail.
3. **Local-first.** The tool operates entirely on a local clone. Collaboration happens through
   standard Git push/fetch.
4. **Structured discussion.** Contributions are typed nodes (claim, objection, summary, etc.)
   rather than opaque comments.
5. **Human-agent parity.** Humans and AI agents use the same CLI surface and identity model.
6. **Policy-driven.** State transitions are gated by configurable guard rules and role
   restrictions.

### 1.2 Implementation constraints

- Language: Rust, stable toolchain.
- Distribution: single `git-forum` binary, installable as a Git subcommand (`git forum`).
- Git integration: subprocess calls to `git` plumbing commands (hash-object, mktree, commit-tree,
  update-ref, rev-list, for-each-ref). No libgit2 dependency.
- Local index: SQLite for search and list views. Rebuildable from refs at any time.

## 2. Core model

### 2.1 Thread kinds

Two thread kinds:

- **rfc** — proposals, designs, and decisions. Lifecycle: `draft` through `accepted` or
  `rejected`.
- **issue** — implementation work items. Lifecycle: `open` through `closed` or `rejected`.

### 2.2 Workflow model

- New projects, features, and design changes start as an RFC.
- Implementation work happens in linked issue threads.
- An accepted RFC, together with its latest summary node, acts as the decision record.
- There is no separate first-class decision object.

### 2.3 Event

An event is an immutable change recorded in a thread's commit history. Each event is stored as a
single Git commit containing an `event.json` blob. The canonical event ID is the Git commit OID
(see ADR-001).

### 2.4 Node

A node is a typed contribution inside a thread. The canonical node ID is the Git commit OID of the
`say` event that introduced the node (see ADR-001).

### 2.5 Evidence

Evidence is a typed reference linking a thread to a commit, file, test, benchmark, document,
another thread, or an external resource.

### 2.6 Actor

An actor is a participant identified by a namespaced string: `human/<name>` or `ai/<name>`.
Resolution order: `--as` flag, then `GIT_FORUM_ACTOR` environment variable, then Git config
`user.name` slugified as `human/<slug>`.

### 2.7 Approval

An approval is a recorded human sign-off attached to a state transition event. The mechanism is
`recorded`; cryptographic signing is a future extension.

## 3. State machines

### 3.1 Issue

```text
open -> pending
open -> closed
open -> rejected
pending -> closed
pending -> open
closed -> open
rejected -> open
```

Initial state: `open`.

`pending` indicates work-in-progress or waiting. `closed` indicates completed work. `rejected`
indicates invalid, duplicate, or won't-fix.

### 3.2 RFC

```text
draft -> proposed
draft -> rejected
proposed -> under-review
proposed -> draft
under-review -> accepted
under-review -> rejected
under-review -> draft
accepted -> deprecated
rejected -> deprecated
```

Initial state: `draft`.

`proposed` means the author declares the RFC review-ready. `under-review` means active review is
in progress. `accepted` is the terminal positive state; an accepted RFC with its latest summary
serves as the decision record. `deprecated` indicates a previously accepted or rejected RFC that
has been superseded or is no longer relevant.

### 3.3 State derivation

State is derived from event replay. The event stream is the authoritative source of truth;
materialized state is a cache.

## 4. Data model

### 4.1 Thread

Required fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Display ID (`RFC-0001`, `ISSUE-0001`) |
| `kind` | enum | `rfc` or `issue` |
| `title` | string | Human-readable title |
| `status` | string | Current state (derived from events) |
| `created_at` | datetime | Creation timestamp |
| `created_by` | string | Actor ID of creator |

Optional fields:

| Field | Type | Description |
|-------|------|-------------|
| `body` | string | Thread body text |
| `scope.branch` | string | Bound Git branch |
| `links[]` | array | Thread-to-thread links |

### 4.2 Event

Required fields:

| Field | Type | Description |
|-------|------|-------------|
| `thread_id` | string | Owning thread display ID |
| `event_type` | enum | Event kind (see below) |
| `created_at` | datetime | Event timestamp |
| `actor` | string | Actor ID |
| `parents[]` | array | Parent commit OIDs |

Conditionally required fields:

| Field | Condition |
|-------|-----------|
| `kind`, `title` | on `create` |
| `node_type`, `body` | on `say` |
| `body` | on `edit` |
| `target_node_id` | on `edit`, `retract`, `resolve`, `reopen` |
| `new_state` | on `state` |
| `approvals[]` | when required by policy |

Event types:

`create`, `say`, `edit`, `retract`, `resolve`, `reopen`, `link`, `state`, `scope`, `verify`,
`merge`.

### 4.3 Node types

| Type | Purpose | Lifecycle |
|------|---------|-----------|
| `claim` | Single assertion or statement | open |
| `question` | Request for information | open |
| `objection` | Blocking concern | open → resolved / retracted |
| `alternative` | Alternative approach | open |
| `evidence` | Supporting evidence reference | open |
| `summary` | Consensus digest | open |
| `action` | Work item | open → resolved / retracted |
| `risk` | Identified risk | open |
| `assumption` | Stated assumption | open |
| `review` | Holistic thread analysis | open (informational) |

`objection` and `action` nodes are open when created. `resolve` closes them; `reopen` reopens
them. `retract` marks any node inactive while preserving history. `incorporated` marks a node as
folded into a thread body revision.

### 4.4 Evidence

Required fields:

| Field | Type | Description |
|-------|------|-------------|
| `evidence_id` | string | Git commit OID of the link event |
| `kind` | enum | Evidence kind |
| `ref` | string | Target reference |

Evidence kinds: `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`, `thread`, `external`.

For `commit` evidence, the ref is resolved to a canonical Git commit OID at write time.

### 4.5 Approval

Required fields:

| Field | Type | Description |
|-------|------|-------------|
| `actor_id` | string | Approving actor |
| `approved_at` | datetime | Approval timestamp |
| `mechanism` | string | `recorded` |

## 5. Storage layout

### 5.1 Git refs

Authoritative data:

```text
refs/forum/threads/<THREAD_ID>    # tip of thread event chain
refs/forum/index/<THREAD_ID>      # rebuildable materialized snapshot
```

### 5.2 Repository files (shared, committed)

```text
.forum/
  policy.toml           # policy configuration
  actors.toml           # registered actors
  templates/
    issue.md            # issue body template
    rfc.md              # RFC body template
```

### 5.3 Local files (per-clone, not committed)

```text
<git-dir>/forum/
  index.sqlite          # search and list index
  local.toml            # local-only settings
  logs/                 # operation logs
```

`<git-dir>` is resolved via `git rev-parse --git-dir`. In a normal repository this is `.git/`. In a
git worktree this is the worktree-specific git directory (e.g.
`/path/to/main/.git/worktrees/<name>/`). This ensures `git-forum` works correctly in worktrees.

## 6. Identity scheme

### 6.1 Thread display IDs

- Issue: `ISSUE-0001`, `ISSUE-0002`, ...
- RFC: `RFC-0001`, `RFC-0002`, ...

Allocated sequentially per kind by scanning existing refs.

### 6.2 Canonical IDs (ADR-001)

- The canonical ID of an event is the Git commit OID that stores it.
- The canonical ID of a node is the Git commit OID of the `say` event that introduced it.

### 6.3 Short-ID resolution

- Full canonical OIDs always accepted.
- Unique prefix of at least 8 hex characters accepted when no exact match exists.
- `node show` resolves globally across all threads.
- Thread-scoped commands (`revise`, `retract`, `resolve`, `reopen`) resolve within the specified
  thread.
- Ambiguous prefixes fail with candidate full IDs listed.

## 7. Policy

Policy is defined in `.forum/policy.toml` and controls:

- **Roles**: which node types an actor may emit (`can_say`) and which state transitions an actor
  may perform (`can_transition`).
- **Guards**: rules that must pass for a given state transition.

### 7.1 Policy file format

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

### 7.2 Guard rules

| Rule | Description |
|------|-------------|
| `no_open_objections` | All objection nodes must be resolved or retracted |
| `no_open_actions` | All action nodes must be resolved or retracted |
| `at_least_one_summary` | At least one summary node must exist |
| `one_human_approval` | At least one `human/*` approval must be attached |
| `has_commit_evidence` | At least one `commit` evidence item must be attached |

### 7.3 Enforcement

- **Guard evaluation**: enforced on `state` command and evaluated read-only by `verify`.
- **Role enforcement**: `can_say` and `can_transition` restrictions are defined in the policy
  schema. Full enforcement is tracked in ISSUE-0023 and ISSUE-0024.

## 8. Concurrency

`git-forum` uses Git's atomic ref updates (compare-and-swap) for write safety:

- `write_event` reads the current thread ref tip, creates a new commit, and atomically updates the
  ref only if the tip has not changed.
- `create_ref` fails if the ref already exists, preventing duplicate thread IDs.
- Concurrent writes to different threads are fully safe.
- Concurrent writes to the same thread fail with a conflict error; the caller retries.

### 8.1 Semantic merge

Semantic merge automatically resolves non-conflicting concurrent writes to the same thread:

**Auto-merge cases** (resolvable without user intervention):

- Concurrent addition of `say` events
- Concurrent addition of evidence
- Concurrent addition of summaries

**Conflict cases** (require user resolution):

- Concurrent terminal state changes on the same thread
- Concurrent `resolve` / `reopen` on the same node

Conflicts surface as unresolved merge state in `show` output.

## 9. CLI surface

### 9.1 Repository setup

```text
git forum init
git forum doctor
git forum reindex
```

### 9.2 Thread creation

```text
git forum issue new <TITLE> [--body <TEXT> | --body-file <PATH> | --body -]
    [--branch <BRANCH>] [--link-to <THREAD_ID> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD_ID>]
git forum rfc new <TITLE> [--body <TEXT> | --body-file <PATH> | --body -]
    [--link-to <THREAD_ID> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD_ID>]
```

Initial states: issue = `open`, RFC = `draft`.

`--from-commit <REV>` populates title from the commit subject, body from the commit message body,
and auto-adds the commit as evidence. An explicit `<TITLE>` argument overrides the subject.

`--from-thread <THREAD_ID>` populates title and body from the source thread and auto-adds a
`relates-to` link to the source thread. An explicit `<TITLE>` argument overrides the source title.

### 9.3 Listing and display

```text
git forum ls [--branch <BRANCH>]
git forum issue ls [--branch <BRANCH>]
git forum issue list                          # alias for ls
git forum rfc ls
git forum show <THREAD_ID>
git forum node show <NODE_ID>
git forum search <QUERY>
git forum status <THREAD_ID>
git forum status --all
git-forum --help-llm                          # works at any subcommand level
```

Thread listings show `YYYY-MM-DD HH:MM` for created and updated timestamps.

### 9.4 Structured discussion

Primitive:

```text
git forum say <THREAD_ID> --type <NODE_TYPE> --body <TEXT>
    [--reply-to <NODE_ID>] [--as <ACTOR>]
```

Shorthand commands:

```text
git forum claim <THREAD_ID> <TEXT>
git forum question <THREAD_ID> <TEXT>
git forum objection <THREAD_ID> <TEXT>
git forum summary <THREAD_ID> <TEXT>
git forum action <THREAD_ID> <TEXT>
git forum risk <THREAD_ID> <TEXT>
git forum review <THREAD_ID> <TEXT>
```

All discussion commands accept `--body`, `--body-file`, `--body -` (stdin), `--reply-to`, and
`--as`.

### 9.5 Node lifecycle

```text
git forum revise <THREAD_ID> <NODE_ID> --body <TEXT>
git forum retract <THREAD_ID> <NODE_ID>
git forum resolve <THREAD_ID> <NODE_ID>
git forum reopen <THREAD_ID> <NODE_ID>
git forum revise-body <THREAD_ID> --body <TEXT> [--incorporates <NODE_ID>]...
```

### 9.6 State changes

Shorthand commands:

```text
git forum issue close <THREAD_ID> [--sign <ACTOR_ID>]...
    [--resolve-open-actions] [--link-to <THREAD_ID> --rel <REL>]
    [--comment <TEXT>]
git forum issue reopen <THREAD_ID> [--comment <TEXT>]
git forum issue reject <THREAD_ID> [--comment <TEXT>]
git forum rfc propose <THREAD_ID> [--comment <TEXT>]
git forum rfc accept <THREAD_ID> [--sign <ACTOR_ID>]...
    [--link-to <THREAD_ID> --rel <REL>] [--comment <TEXT>]
git forum rfc deprecate <THREAD_ID> [--comment <TEXT>]
    [--link-to <THREAD_ID> --rel <REL>]
```

`--comment` adds a summary node before the state transition.
`--link-to` creates thread links after the state transition.

Generic state command:

```text
git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]...
    [--resolve-open-actions] [--link-to <THREAD_ID> --rel <REL>]
    [--comment <TEXT>]
git forum state bulk --to <NEW_STATE> [<THREAD_ID>...] [--branch <BRANCH>]
    [--kind <KIND>] [--status <STATUS>] [--sign <ACTOR_ID>]...
    [--resolve-open-actions] [--dry-run]
```

### 9.7 Evidence and links

```text
git forum evidence add <THREAD_ID> --kind <KIND> --ref <REF> [<REF>...]
git forum link <FROM> <TO> --rel <REL>
git forum branch bind <THREAD_ID> <BRANCH>
git forum branch clear <THREAD_ID>
```

`--ref` accepts multiple values. Each ref creates a separate evidence event.

Common link relations: `implements`, `relates-to`, `depends-on`, `blocks`.

### 9.8 Verification and policy

```text
git forum verify <THREAD_ID>
git forum policy lint
git forum policy check <THREAD_ID> --transition <TRANSITION>
```

`verify` is read-only. It evaluates forward-transition guards:

- RFC in `under-review` → checks `under-review->accepted` guards.
- Issue in `open` → checks `open->closed` guards.

### 9.9 TUI

```text
git forum tui [<THREAD_ID>]
```

### 9.10 Import / export

```text
git forum import github-issue <SOURCE>
git forum import markdown-rfc <PATH>
git forum export <THREAD_ID> [--format <FORMAT>]
```

## 10. Command requirements

### 10.1 `issue new` and `rfc new`

- Create a `create` event.
- Accept `--body`, `--body-file`, or `--body -` (stdin).
- Accept `--link-to <THREAD_ID> --rel <REL>`.
- Accept `--branch <BRANCH>` (issue only).
- Accept `--from-commit <REV>`: populate title/body from commit message, auto-add commit evidence.
- Accept `--from-thread <THREAD_ID>`: populate title/body from source thread, auto-add `relates-to`
  link.
- Title accepts values starting with hyphens (`allow_hyphen_values`).
- Allocate a display ID.
- Assign the initial state.
- Update the thread ref.

### 10.2 `say` and shorthand commands

- Append a `say` event.
- Validate node type against policy role restrictions.
- `--reply-to <NODE_ID>` links the new node as a response.
- Actor resolution: `--as` flag > `GIT_FORUM_ACTOR` env var > Git config `user.name`.

### 10.3 `revise`, `retract`, `resolve`, `reopen`

- `revise` appends an `edit` event.
- `retract` appends a `retract` event.
- `resolve` and `reopen` operate primarily on `objection` and `action` nodes.
- Node commands accept full canonical IDs and unique prefixes (minimum 8 characters).

### 10.4 `show`

Display:

1. Title, body, kind, status.
2. Branch scope.
3. Body revision count (if revised).
4. Incorporated nodes (if any).
5. Open objections.
6. Open actions.
7. Latest summary.
8. Evidence items.
9. Thread links.
10. Conversations (reply chains grouped by root node).
11. Timeline in `date node_id event_id author type body` order.

### 10.5 `state` and shorthand commands

- Validate the transition against the state machine.
- Evaluate guard rules from policy.
- Validate actor role has `can_transition` permission.
- `--comment <TEXT>` creates a summary node before the state transition.
- Attach approvals from `--sign`.
- Append a `state` event.
- `--link-to <THREAD_ID> --rel <REL>` creates thread links after the state transition.
- `--resolve-open-actions` resolves open action nodes before the transition.
- Shorthand commands: `issue close`, `issue reopen`, `issue reject`, `rfc propose`, `rfc accept`,
  `rfc deprecate`.

### 10.6 `state bulk`

- Evaluate each target independently.
- Allow target selection by explicit IDs or `--branch` / `--kind` / `--status`.
- Apply successful transitions; report failures inline.
- Exit non-zero if any target failed.
- `--dry-run` reports outcomes without writing events.

### 10.7 `evidence add` and `link`

- `evidence add` appends a `link` event carrying evidence metadata.
- `--ref` accepts multiple values; each creates a separate evidence event.
- `--kind commit --ref <REV>` resolves `<REV>` to a canonical commit OID.
- `link` records thread-to-thread relations.

### 10.8 `verify`

Read-only guard evaluation:

- Guard violations.
- Missing summary before RFC acceptance.
- Unresolved objections before RFC acceptance.
- Unresolved actions before issue close.

## 11. TUI

The TUI is a terminal UI for browsing and basic creation.

### 11.1 Views

1. Thread list with kind filter.
2. Thread detail with node list.
3. Node detail with history.
4. Thread create form.
5. Node create form.
6. Thread link create form.

### 11.2 Capabilities

- Thread list: sort by column, filter by kind, refresh from index.
- Node lifecycle: resolve, reopen, retract from node detail view.
- Mouse: single click selects, double click opens, scroll wheel, click-to-sort column headers,
  click submit buttons.
- Color coding: thread kind, thread status, node type, node status. Dim resolved, retracted, and
  incorporated rows.
- Keyboard: `j`/`k` navigation, `enter` to open, `c` to create, `q`/`esc` to go back.

### 11.3 Boundary

Policy-sensitive operations (state changes with guards, approvals) remain CLI-first.

## 12. Search

Lexical search over the SQLite index.

Search surface:

- Thread title and body.
- Thread kind and status.
- Thread ID and branch.
- Current node body.
- Node type and node ID.

Results are grouped by thread. Node-level matches indicate which node matched.

## 13. Error handling

The CLI returns:

- A specific failure reason.
- The violated guard, policy rule, or state-machine rule.
- A hint about the next valid move when helpful.

Examples:

```text
transition draft->under-review is not valid for rfc; valid transitions from 'draft': [proposed, rejected]
FAIL [no_open_objections] unresolved objections remain
```

## 14. Testing strategy

### 14.1 Test layers

1. Unit tests: replay, state machine, policy, guard, merge, index, search.
2. Integration tests: temporary Git repositories with isolated config.
3. TUI tests: non-interactive backend.
4. Snapshot tests: deterministic output for `show`, `verify`, `ls`, `status`.

### 14.2 Test requirements

- No dependency on global Git config (`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`).
- No network access.
- Clock and ID generation are replaceable (`FixedClock`, `StepClock`, `SequentialIdGenerator`).

### 14.3 CI baseline

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

## 15. Non-goals

The following are explicitly out of scope:

- A separate `decision` thread kind.
- Mandatory AI provenance tracking.
- AI-only high-level command sets.
- A Web UI.
- A central server.
- Real-time collaboration.
- Advanced access control beyond role-based policy.
- Automatic patch application.
- Large PM-style workflow systems.
- Embedding-based recommendation or semantic search.
