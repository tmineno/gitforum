# git-forum Product Specification

Version 1.2 — 2026-03-21

## 1. Overview

`git-forum` is a Git-native structured discussion and work-tracking tool for human-agent software
development. It stores RFCs, decision records, tasks, issues, typed discussion, evidence, and state
transitions as append-only events inside a Git repository.

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

Four thread kinds:

- **rfc** — cross-cutting proposals and designs. Lifecycle: `draft` through `accepted` or
  `rejected`.
- **dec** — local design decisions worth recording. Lifecycle: `proposed` through `accepted` or
  `rejected`.
- **task** — implementable work units with phase discipline. Lifecycle: `open` through `closed` or
  `rejected`, with `designing`, `implementing`, and `reviewing` phases.
- **issue** — bug reports and feature requests. Lifecycle: `open` through `closed` or `rejected`.

### 2.2 Workflow model

- Cross-cutting design decisions start as an RFC.
- Local design decisions that need to survive beyond a PR are recorded as DECs.
- Implementable work is tracked as TASKs with phase discipline.
- Bugs and feature requests are tracked as ISSUEs.
- Agents are participants, not a separate control plane.

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

### 3.3 DEC

```text
proposed -> accepted
proposed -> rejected
proposed -> deprecated
accepted -> deprecated
rejected -> deprecated
```

Initial state: `proposed`.

`proposed` means the decision is under consideration. `accepted` means the decision is ratified.
`rejected` means the decision was not adopted. `deprecated` indicates a decision that has been
superseded or is no longer relevant. `proposed -> deprecated` allows archiving a decision that
becomes moot before acceptance or rejection.

### 3.4 TASK

```text
open -> designing
open -> rejected
open -> closed
designing -> implementing
designing -> rejected
designing -> open
implementing -> reviewing
implementing -> rejected
implementing -> designing
reviewing -> closed
reviewing -> rejected
reviewing -> implementing
closed -> open
rejected -> open
```

Initial state: `open`.

`designing`, `implementing`, and `reviewing` are phase states that track progress. `open -> closed`
is a fast-track for trivial tasks that need no design/review phase. Back-transitions skip at most
one phase (e.g., `reviewing -> implementing` is valid but `reviewing -> designing` is not).
`rejected` can be reached from any active phase. Reopen always returns to `open`.

### 3.5 State derivation

State is derived from event replay. The event stream is the authoritative source of truth;
materialized state is a cache.

## 4. Data model

### 4.1 Thread

Required fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Display ID (`RFC-0001`, `ISSUE-0001`, `DEC-0001`, `TASK-0001`) |
| `kind` | enum | `rfc`, `issue`, `dec`, or `task` |
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
`merge`, `revise-body`.

### 4.3 Node types

| Type | Purpose | Lifecycle |
|------|---------|-----------|
| `claim` | Single assertion or statement | open |
| `question` | Request for information | open |
| `objection` | Blocking concern | open → resolved / retracted |
| `evidence` | Supporting evidence reference | open |
| `summary` | Consensus digest | open |
| `action` | Work item | open → resolved / retracted |
| `risk` | Identified risk | open |
| `review` | Holistic thread analysis | open (informational) |
| `alternative` | Considered alternative approach | open |
| `assumption` | Design dependency or assumption | open |

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
```

### 5.2 Repository files (shared, committed)

```text
.forum/
  policy.toml           # policy configuration
  actors.toml           # registered actors
  templates/
    issue.md            # issue body template
    rfc.md              # RFC body template
    dec.md              # DEC body template
    task.md             # TASK body template
```

### 5.3 Local files (per-clone, not committed)

```text
<git-dir>/forum/
  index.db              # search and list index (SQLite)
  local.toml            # local-only settings
  logs/                 # operation logs
```

`<git-dir>` is resolved via `git rev-parse --git-dir`. In a normal repository this is `.git/`. In a
git worktree this is the worktree-specific git directory (e.g.
`/path/to/main/.git/worktrees/<name>/`). This ensures `git-forum` works correctly in worktrees.

#### 5.3.1 `local.toml` format

```toml
# Actor used when --as is not specified and GIT_FORUM_ACTOR is unset.
default_actor = "human/alice"

# Override Git commit author/committer metadata on forum commits.
# Both fields are optional; unset fields fall through to git config.
[commit_identity]
name = "alice"
email = "alice@example.com"
```

**`default_actor`** — sets the default actor ID for this clone. Overridden by `--as` and
`GIT_FORUM_ACTOR`.

**`[commit_identity]`** — controls the Git commit metadata (author name and email) used when
writing forum events. This is separate from the actor ID stored in `event.json`. Resolution order:

1. `local.toml` `[commit_identity]` name/email (if set)
2. Git config `user.name` / `user.email` (default)

## 6. Identity scheme

### 6.1 Thread display IDs

- Issue: `ISSUE-0001`, `ISSUE-0002`, ...
- RFC: `RFC-0001`, `RFC-0002`, ...
- DEC: `DEC-0001`, `DEC-0002`, ...
- TASK: `TASK-0001`, `TASK-0002`, ...

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

- **Guards**: rules that must pass for a given state transition.
- **Operation checks**: rules that validate write operations before committing events.

### 7.1 Policy file format

The default policy shipped by `git forum init`:

```toml
[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]

[[guards]]
on = "proposed->accepted"
requires = ["no_open_objections"]

[[guards]]
on = "reviewing->closed"
requires = ["no_open_actions"]

[checks]
strict = false

[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[creation_rules.issue]
required_body = false
body_sections = []

[creation_rules.dec]
required_body = true
body_sections = ["Context", "Decision", "Rationale", "Impact"]

[creation_rules.task]
required_body = false
body_sections = ["Background", "Acceptance criteria", "Exceptions"]

[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending", "designing", "implementing"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing"]

[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing", "closed", "accepted", "rejected", "deprecated"]
```

### 7.2 Guard rules

| Rule | Description |
|------|-------------|
| `no_open_objections` | All objection nodes must be resolved or retracted |
| `no_open_actions` | All action nodes must be resolved or retracted |
| `at_least_one_summary` | At least one summary node must exist |
| `one_human_approval` | At least one `human/*` approval must be attached |
| `has_commit_evidence` | At least one `commit` evidence item must be attached |

#### Kind-scoped guard keys

The `on` field supports an optional thread-kind prefix:

```toml
[[guards]]
on = "dec:proposed->accepted"       # only applies to DEC threads
requires = ["no_open_objections"]

[[guards]]
on = "proposed->accepted"           # applies to all kinds with this transition
requires = ["no_open_objections"]
```

- Scoped format: `"<kind>:<from>-><to>"` — guard fires only for the specified kind.
- Unscoped format: `"<from>-><to>"` — guard fires for every kind that has the transition.
- When both a scoped and unscoped guard match, both apply (union semantics).

### 7.3 Operation checks

Operation checks validate write commands against policy before events are committed.

```toml
[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Design", "Failure modes", "Acceptance tests"]

[creation_rules.issue]
required_body = false
body_sections = []

[node_rules]
"draft" = ["claim", "question", "objection", "evidence", "summary", "action", "risk", "review"]
"accepted" = []

[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending"]

[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending"]

[checks]
strict = false
```

Check functions:

| Function | Commands | Validates |
|----------|----------|-----------|
| `check_create` | `new` | Required body, required body sections (markdown headings) |
| `check_say` | node commands | Node type allowed in the current state |
| `check_revise` | `revise` | Revision allowed in the current state |
| `check_evidence` | `evidence add` | Evidence addition allowed in the current state |

Each returns `Vec<OperationViolation>` (pure function, no I/O).

Severity model:

| Severity | Behavior | `--force` effect |
|----------|----------|------------------|
| Error | Always blocks | Cannot bypass |
| Warning | Printed to stderr, operation proceeds | N/A (already proceeds) |
| Warning + `strict = true` | Becomes error (blocks) | Downgrades back to warning |

Specific assignments:

- Missing body when `required_body = true`: **Error**
- Missing or empty required body section: **Warning**
- Node type not allowed in state: **Error**
- Revision not allowed in state: **Error**
- Evidence not allowed in state: **Error**

Missing policy file or missing sections apply no restrictions (`#[serde(default)]`).

### 7.4 Enforcement

- **Guard evaluation**: enforced on `state` command and evaluated read-only by `verify`.
- **Operation checks**: enforced on `new`, node commands (`claim`, `question`, etc.), `revise`,
  and `evidence add`. All write commands accept `--force` to bypass warning-level violations.
  See `doc/spec/operation-checks.md` for details.

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
git forum new issue <TITLE> [--body <TEXT> | --body-file <PATH> | --body - | --edit]
    [--branch <BRANCH>] [--link-to <THREAD_ID> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD_ID>] [--force]
git forum new rfc <TITLE> [--body <TEXT> | --body-file <PATH> | --body - | --edit]
    [--link-to <THREAD_ID> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD_ID>] [--force]
git forum new dec <TITLE> [--body <TEXT> | --body-file <PATH> | --body - | --edit]
    [--link-to <THREAD_ID> --rel <REL>] [--force]
git forum new task <TITLE> [--body <TEXT> | --body-file <PATH> | --body - | --edit]
    [--branch <BRANCH>] [--link-to <THREAD_ID> --rel <REL>] [--force]
```

The old forms `git forum issue new`, `git forum rfc new`, etc. remain as hidden aliases for backward
compatibility.

Initial states: issue = `open`, RFC = `draft`, DEC = `proposed`, TASK = `open`.

`--from-commit <REV>` populates title from the commit subject, body from the commit message body,
and auto-adds the commit as evidence. An explicit `<TITLE>` argument overrides the subject.

`--from-thread <THREAD_ID>` populates title (prefixed with `v2: `) and body from the source thread
and creates bidirectional `supersedes` / `superseded-by` links. An explicit `<TITLE>` argument
overrides the default title. Allowed combinations:

- **RFC → new RFC**: source RFC is auto-deprecated.
- **Issue → new issue**: source issue state is unchanged.
- **Issue → new RFC**: source issue state is unchanged.
- **RFC → new issue**: rejected with an error (use `link --rel implements` instead).

### 9.3 Listing and display

```text
git forum ls [--kind <KIND>] [--branch <BRANCH>]
git forum show <THREAD_ID>
git forum node show <NODE_ID>
git forum search <QUERY>
git forum status <THREAD_ID>
git-forum --help-llm                          # works at any subcommand level
```

`--kind rfc`, `--kind issue`, `--kind dec`, or `--kind task` filters the listing by thread kind. The
old forms `git forum issue ls`, `git forum rfc ls`, etc. remain as hidden aliases for backward
compatibility.

Thread listings show `YYYY-MM-DD HH:MM` for created and updated timestamps.

### 9.4 Structured discussion

```text
git forum claim <THREAD_ID> <TEXT> [--edit] [--force]
git forum question <THREAD_ID> <TEXT> [--edit] [--force]
git forum objection <THREAD_ID> <TEXT> [--edit] [--force]
git forum summary <THREAD_ID> <TEXT> [--edit] [--force]
git forum action <THREAD_ID> <TEXT> [--edit] [--force]
git forum risk <THREAD_ID> <TEXT> [--edit] [--force]
git forum review <THREAD_ID> <TEXT> [--edit] [--force]
git forum node add <THREAD_ID> --type <TYPE> <TEXT> [--edit] [--force]
```

All discussion commands accept a positional body argument (use `"-"` to read from stdin),
`--body-file`, `--edit`, `--reply-to`, `--as`, and `--force`.

`node add` is a generic interface for all node types, including `alternative` and `assumption` which
have no dedicated shorthand.

### 9.5 Node lifecycle

```text
git forum revise <THREAD_ID> [--body <TEXT> | --body-file <PATH> | --edit] [--incorporates <NODE_ID>]... [--force]
git forum revise body <THREAD_ID> [--body <TEXT> | --body-file <PATH> | --edit] [--incorporates <NODE_ID>]... [--force]
git forum revise node <THREAD_ID> <NODE_ID> [--body <TEXT> | --body-file <PATH> | --edit] [--force]
git forum retract <THREAD_ID> <NODE_ID>...
git forum resolve <THREAD_ID> <NODE_ID>...
git forum reopen <THREAD_ID> <NODE_ID>...      # node reopen (with node IDs)
```

`git forum revise` without a subcommand defaults to body revision. The explicit `revise body`
form continues to work.

`git forum retract`, `resolve`, and `reopen` accept one or more node IDs. Each node is processed
independently; failures are reported inline on stderr and the command exits non-zero if any fail.

`git forum reopen` with node IDs reopens those nodes. Without node IDs (thread ID only) it
performs a thread state reopen (see section 9.6).

### 9.6 State changes

Shorthand commands (top-level, verb-first):

```text
git forum close <THREAD_ID> [--approve <ACTOR_ID>]...
    [--resolve-open-actions] [--link-to <THREAD_ID> --rel <REL>]
    [--comment <TEXT>]
git forum pend <THREAD_ID> [--comment <TEXT>]
git forum reopen <THREAD_ID> [--comment <TEXT>]
git forum reject <THREAD_ID> [--comment <TEXT>]
git forum propose <THREAD_ID> [--comment <TEXT>]
git forum accept <THREAD_ID> [--approve <ACTOR_ID>]...
    [--link-to <THREAD_ID> --rel <REL>] [--comment <TEXT>]
git forum deprecate <THREAD_ID> [--comment <TEXT>]
```

`git forum reopen` with one argument (thread ID only) performs a thread state reopen. With two
arguments (thread ID + node ID) it reopens a node (see section 9.5).

`--comment` attaches comment text to the state-change event's body (visible in the timeline).
`--link-to` creates thread links after the state transition.

The old kind-prefixed forms (`git forum issue close`, `git forum rfc accept`, etc.) remain as hidden
aliases for backward compatibility.

Generic state command:

```text
git forum state <THREAD_ID> <NEW_STATE> [--approve <ACTOR_ID>]...
    [--resolve-open-actions] [--link-to <THREAD_ID> --rel <REL>]
    [--comment <TEXT>]
git forum state bulk --to <NEW_STATE> [<THREAD_ID>...] [--branch <BRANCH>]
    [--kind <KIND>] [--status <STATUS>] [--approve <ACTOR_ID>]...
    [--resolve-open-actions] [--dry-run]
```

### 9.7 Evidence and links

```text
git forum evidence add <THREAD_ID> --kind <KIND> --ref <REF> [<REF>...] [--force]
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
- DEC in `proposed` → checks `proposed->accepted` guards.
- TASK in `reviewing` → checks `reviewing->closed` guards.

### 9.9 TUI

```text
git forum tui [<THREAD_ID>]
```

### 9.10 Hooks

```text
git forum hook install [--force]
git forum hook uninstall
git forum hook check-commit-msg <FILE>
```

`git forum init` auto-installs the commit-msg hook. The hook validates that thread IDs referenced
in commit messages (`ISSUE-NNNN`, `RFC-NNNN`, `DEC-NNNN`, `TASK-NNNN`) exist as git-forum refs. Comment lines (respecting
`core.commentChar`) and scissors sections are stripped before scanning.

- No thread IDs found: warn, exit 0 (non-blocking).
- All referenced threads exist: exit 0.
- Any missing: warn, exit 1 (blocks commit).

Hook path is resolved via `git rev-parse --git-path hooks/commit-msg` (worktree and
`core.hooksPath` safe). `--force` overwrites without backup.

### 9.11 Import / export (planned)

Import and export commands are planned but not yet implemented:

```text
git forum import github-issue --repo <OWNER/REPO> --issue <NUMBER>
git forum export github-issue <THREAD_ID> --repo <OWNER/REPO>
```

Scope: GitHub issue interoperability only. RFC import/export is not planned.

## 10. Command requirements

### 10.1 `new issue` and `new rfc`

- Create a `create` event.
- Accept `--body`, `--body-file`, `--body -` (stdin), or `--edit` (open `$EDITOR`).
- `--edit` opens `$VISUAL` / `$EDITOR` / `vi` for interactive composition; conflicts with `--body`
  and `--body-file`. Empty content aborts the command.
- Accept `--link-to <THREAD_ID> --rel <REL>`.
- Accept `--branch <BRANCH>` (issue only).
- Accept `--from-commit <REV>`: populate title/body from commit message, auto-add commit evidence.
- Accept `--from-thread <THREAD_ID>`: populate title/body from source thread, add bidirectional
  `supersedes` / `superseded-by` links, auto-deprecate source only if RFC→RFC. Reject RFC→issue.
- Accept `--force`: bypass warning-level operation check violations (does not bypass errors).
- Evaluate `check_create` against `[creation_rules]` policy before committing.
- Title accepts values starting with hyphens (`allow_hyphen_values`).
- Allocate a display ID.
- Assign the initial state.
- Update the thread ref.

### 10.2 Node commands

- Append a `say` event.
- Evaluate `check_say` against `[node_rules]` policy before committing.
- Accept `--force`: bypass warning-level operation check violations (does not bypass errors).
- Accept `--edit`: open `$EDITOR` for interactive body composition; conflicts with positional body,
  `--body`, and `--body-file`.
- `--reply-to <NODE_ID>` links the new node as a response.
- Actor resolution: `--as` flag > `GIT_FORUM_ACTOR` env var > Git config `user.name`.

### 10.3 `revise`, `retract`, `resolve`, `reopen`

- `revise` appends an `edit` event. Without a subcommand, `revise` defaults to body revision.
  The explicit `revise body` and `revise node` forms continue to work.
- `revise` evaluates `check_revise` against `[revise_rules]` policy before committing.
- `revise` accepts `--force`: bypass warning-level operation check violations (does not bypass errors).
- `revise` accepts `--edit`: open `$EDITOR` for interactive body composition; conflicts with
  `--body` and `--body-file`.
- `retract` appends a `retract` event.
- `resolve` and `reopen` operate primarily on `objection` and `action` nodes.
- `retract`, `resolve`, and `reopen` accept one or more node IDs. Each node is processed
  independently; failures are reported inline and the command exits non-zero if any fail.
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
- `--comment <TEXT>` attaches comment text to the state-change event's body.
- Attach approvals from `--approve`.
- Append a `state` event.
- `--link-to <THREAD_ID> --rel <REL>` creates thread links after the state transition.
- `--resolve-open-actions` resolves open action nodes before the transition.
- Shorthand commands: `close`, `pend`, `reopen`, `reject`, `propose`, `accept`, `deprecate`.
  The old kind-prefixed forms (`issue close`, `rfc accept`, etc.) remain as hidden aliases.

### 10.6 `state bulk`

- Evaluate each target independently.
- Allow target selection by explicit IDs or `--branch` / `--kind` / `--status`.
- Apply successful transitions; report failures inline.
- Exit non-zero if any target failed.
- `--dry-run` reports outcomes without writing events.

### 10.7 `evidence add` and `link`

- `evidence add` appends a `link` event carrying evidence metadata.
- `evidence add` evaluates `check_evidence` against `[evidence_rules]` policy before committing.
- `evidence add` accepts `--force`: bypass warning-level operation check violations (does not bypass errors).
- `--ref` accepts multiple values; each creates a separate evidence event.
- `--kind commit --ref <REV>` resolves `<REV>` to a canonical commit OID.
- `link` records thread-to-thread relations.

### 10.8 `verify`

Read-only guard evaluation:

- Guard violations.
- Missing summary before RFC acceptance.
- Unresolved objections before RFC acceptance.
- Unresolved actions before issue close.

### 10.9 `hook`

- `hook install` writes a shell script to the hooks directory, makes it executable.
- If hook already contains the git-forum marker: print "already installed", succeed.
- If hook exists without marker and no `--force`: fail with suggestion to use `--force`.
- `--force` overwrites without backup.
- `hook uninstall` removes the hook only if it matches the git-forum template.
- `hook check-commit-msg <FILE>`:
  - Query `core.commentChar` (default `#`), strip comment lines and scissors sections.
  - Extract thread IDs matching known prefixes (`ISSUE`, `RFC`, `DEC`, `TASK`) with 4-digit suffixes.
  - Validate each ID against `refs/forum/threads/<ID>`.
  - No IDs found: warn, exit 0. All valid: exit 0. Any missing: warn, exit 1.

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

- A separate heavyweight decision workflow (DEC is lightweight by design).
- Mandatory AI provenance tracking.
- AI-only high-level command sets.
- A Web UI.
- A central server.
- Real-time collaboration.
- Advanced access control beyond role-based policy.
- Automatic patch application.
- Large PM-style workflow systems.
- Embedding-based recommendation or semantic search.
