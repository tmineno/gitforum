# git-forum Product Specification - 3.0 Draft

Version 3.0-draft - 2026-05-03 (JST)
Status: **Draft**. This document is intended to be self-contained: every
normative 3.0 rule is written here directly rather than inherited by reference
from an earlier specification.
Discussion thread: `@fg61bcmp`.
Bound by `doc/spec/CORE-VALUE.md` - when this document conflicts with the core
value statement, this document is wrong and must be revised.

> 3.0 uses a `thread` model classified by `lifecycle` and `tags`, with four
> canonical node types: `comment`, `approval`, `objection`, and `action`.
> Storage changes from event replay to **snapshot refs**.
>
> Authoritative forum data still lives under `refs/forum/*`. This preserves
> the property that local forum issues are unified across branch switches and
> linked worktrees. What changes is the object stored at each thread ref: the
> ref points to a snapshot tree, not to an append-only domain-event chain.

## 1. Overview

### 1.1 Key changes from the event-chain design

| Concern | Event-chain design (2.x or earlier) | 3.0 draft |
|---|---|---|
| Authoritative namespace | `refs/forum/*` | `refs/forum/*`; forum data remains branch-independent. |
| Thread ref content | Event commit chain; replay builds current state | Snapshot commit; tip tree is current state |
| Read path | Load all events, validate, replay | Read ref tip tree and parse snapshot files |
| History model | Domain-event timeline owned by git-forum | Git commit history and file diffs |
| Repair model | Orphan/stale event repair, strict replay diagnostics | Snapshot schema and reference integrity checks |
| Migration | 1.x/2.x event rewrite | 1.x/2.x event chain -> 3.0 snapshot commit |
| Compatibility | Legacy event/state/node compatibility in runtime | One-shot migration; legacy events are archival, not the live model |
| Policy grammar | Facet expression strings | Structured TOML selectors; no custom boolean expression parser |

### 1.2 Design principles

1. **Branch-independent forum state.** Thread data MUST NOT be stored as ordinary
   tracked files on the user's current source branch. It MUST remain under
   `refs/forum/*`.
2. **Snapshot as source of truth.** The current state of a thread is represented
   directly in the tip tree of `refs/forum/threads/<thread-id>`.
3. **Git owns history.** git-forum does not maintain a separate domain-event
   database. `git log`, `git diff`, and normal Git object reachability are the
   history substrate.
4. **Compatibility is a migration concern.** 3.0 runtime code should not keep
   1.x/2.x event replay semantics alive except in the migration tool.
5. **Smaller core through one application command layer.** CLI and TUI are both
   required 3.0 clients, but they MUST share the same application command layer
   for reads, validation, policy checks, and snapshot writes. GitHub bridges,
   advanced search, and compatibility shims are not core requirements unless
   they reuse that layer without duplicating write logic.

## 2. Core model

### 2.1 Thread

A thread is the primary discussion unit: one focused proposal, task, bug,
question, decision record, or other unit of repository discussion. The live
thread state is a snapshot with these required fields:

| Field | Type | Description |
|---|---|---|
| `schema_version` | integer | Must be `3` for native 3.0 snapshots. |
| `id` | string | Thread ID, storage form without display marker. |
| `title` | string | Human-readable title. |
| `lifecycle` | enum | `proposal`, `execution`, or `record`. |
| `status` | enum | Current state from the unified state machine in §3.1. |
| `tags` | array | Free-form tags using the grammar in §2.4. |
| `created_at` | datetime | Creation timestamp. |
| `created_by` | string | Actor ID. |
| `updated_at` | datetime | Timestamp of last snapshot update. |
| `updated_by` | string | Actor ID for last snapshot update. |

Optional fields:

| Field | Type | Description |
|---|---|---|
| `branch` | string | Branch scope advisory. |
| `supersedes` | array | Convenience summary of outgoing `supersedes` links, if implementations choose to denormalize. |

The thread body is stored separately as `body.md` so ordinary Git diffs are useful.

### 2.2 Node

3.0 has four canonical node types:

| Node type | Protocol effect |
|---|---|
| `comment` | Body-prose contribution. |
| `approval` | Positive sign-off; may satisfy approval guards. |
| `objection` | Blocking concern until resolved. |
| `action` | Tracked work item until resolved. |

Live 3.0 snapshots do not have `claim`, `question`, `summary`, `risk`,
`review`, `alternative`, `assumption`, or `legacy_subtype` as first-class node
types. Migration MAY preserve those labels in an archival field, but core
behavior MUST use the canonical four.

Each node has metadata and body files:

```text
nodes/<node-id>.toml
nodes/<node-id>.md
```

Required metadata fields:

| Field | Type | Description |
|---|---|---|
| `id` | string | Path-safe opaque node ID. |
| `type` | enum | `comment`, `approval`, `objection`, or `action`. |
| `status` | enum | `open`, `resolved`, `retracted`, or `incorporated`. |
| `created_at` | datetime | Creation timestamp. |
| `created_by` | string | Actor ID. |

Optional metadata fields:

| Field | Type | Description |
|---|---|---|
| `updated_at` | datetime | Last edit timestamp. |
| `updated_by` | string | Last edit actor. |
| `reply_to` | string | Parent node ID in the same thread. |
| `legacy_label` | string | Migration-only archival label; ignored for live behavior. |

### 2.3 Links and evidence

Thread links remain the grouping mechanism. There is no topic entity.

`links.toml` stores current outgoing links:

```toml
[[links]]
target = "abc123xy"
rel = "implements"
created_at = "2026-05-03T00:00:00Z"
created_by = "human/alice"
```

Common relation values are `implements`, `relates-to`, `depends-on`, `blocks`,
`supersedes`, and `superseded-by`. Repositories MAY use additional lowercase
relation strings following the tag character rules in §2.4.

The group associated with a parent thread is defined narrowly as direct incoming
links with `rel = "implements"`. `git forum show <THREAD> --tree` MAY display
those children as an advisory one-hop tree. It MUST NOT recurse, include other
relations by default, or gate any state transition.

`evidence.toml` stores current evidence records:

```toml
[[evidence]]
id = "ev1"
kind = "commit"
ref = "HEAD"
created_at = "2026-05-03T00:00:00Z"
created_by = "human/alice"
```

Evidence IDs are opaque and local to the thread snapshot.

Evidence `kind` values:

| Kind | `ref` meaning |
|---|---|
| `commit` | Git commit, branch, tag, or revision. |
| `file` | Repository-relative file path. |
| `hunk` | File path plus line or range selector. |
| `test` | Test file, test command, or suite identifier. |
| `benchmark` | Benchmark result path or identifier. |
| `doc` | Documentation path or identifier. |
| `thread` | Another git-forum thread ID. |
| `external` | External URL or opaque external reference. |

### 2.4 Lifecycle and tag vocabulary

`lifecycle` is the only required classification facet. It controls which states
are valid for a thread.

| Lifecycle | Meaning |
|---|---|
| `proposal` | A proposal that may be drafted, opened for review, accepted, rejected, withdrawn, or deprecated. |
| `execution` | Work or bug tracking that may move through open, working, review, done, rejected, or deprecated states. |
| `record` | A short-lived record or decision that usually moves from open to done or rejected. |

`tags` are free-form labels for subcategories such as `bug`, `task`, or
`cross-cutting`. Tags are first-class for filtering and policy selectors, but
there is no required tag registry in 3.0.

Every tag MUST satisfy:

- ASCII lowercase only: `[a-z0-9-]`.
- Starts with a lowercase letter: `[a-z]`.
- Length is 2 to 32 characters.
- Not equal to `all`, `none`, `any`, or `untagged`.
- Contains no spaces, slashes, colons, `@`, or `!`.

The preset commands in §7 emit these conventional tags:

| Preset | Lifecycle | Tags |
|---|---|---|
| `new rfc` | `proposal` | `cross-cutting` |
| `new dec` | `record` | none |
| `new task` | `execution` | `task` |
| `new issue` | `execution` | `bug` |
| `new bug` | `execution` | `bug` |

## 3. State machine and policy

### 3.1 State machine

A single transition graph is used for every thread. A transition is valid only
when both the edge exists in this graph and the destination state is allowed for
the thread's lifecycle.

Unified transition graph:

```text
draft    -> open
draft    -> withdrawn
open     -> working
open     -> review
open     -> done
open     -> rejected
open     -> withdrawn
working  -> review
working  -> done
working  -> rejected
review   -> done
review   -> working
review   -> rejected
done     -> open
rejected -> open
done     -> deprecated
rejected -> deprecated
```

Lifecycle-filtered states:

| Lifecycle | Allowed states | Initial state | Typical path |
|---|---|---|---|
| `proposal` | `draft`, `open`, `review`, `done`, `rejected`, `withdrawn`, `deprecated` | `draft` | `draft -> open -> review -> done` |
| `execution` | `open`, `working`, `review`, `done`, `rejected`, `deprecated` | `open` | `open -> working -> review -> done`; trivial work may use `open -> done` |
| `record` | `open`, `done`, `rejected`, `deprecated` | `open` | `open -> done` |

`withdrawn` and `deprecated` are absorbing terminal states. Terminal states for
filtering are `done`, `rejected`, `deprecated`, and `withdrawn`.

A transition whose destination state is not allowed for the thread's lifecycle
MUST fail with a lifecycle/state mismatch diagnostic that names the lifecycle,
the rejected destination, and the allowed state set.

State transitions update `thread.toml` directly. The historical record of a
transition is the Git commit that changed `status`, plus the commit message and
diff. There is no separate `state` event object in the live model.

### 3.2 Structured policy selectors

3.0 replaces string facet expressions with structured TOML selectors.

Old string-expression form:

```toml
[[guards]]
on = "lifecycle=proposal AND tag=cross-cutting : review->done"
requires = ["one_human_approval", "no_open_objections"]
```

3.0:

```toml
[[guards]]
transition = "review->done"
lifecycle = "proposal"
tags_all = ["cross-cutting"]
requires = ["one_human_approval", "no_open_objections"]
```

Selector fields:

| Field | Type | Meaning |
|---|---|---|
| `transition` | string | Required `from->to` transition. |
| `lifecycle` | string | Optional lifecycle equality match. |
| `tags_all` | array | Optional set of tags all required to match. |
| `tags_any` | array | Optional set of tags where at least one must match. |
| `tags_none` | array | Optional set of tags that must not be present. |

All present selectors are combined with AND semantics. This intentionally avoids
a custom boolean parser in the core.

### 3.3 Operation checks

Operation checks use the same structured selector shape:

```toml
[[creation_rules]]
lifecycle = "proposal"
tags_all = ["cross-cutting"]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]
```

Most-specific match wins. If multiple rules tie, the implementation MUST fail
with an ambiguous-policy diagnostic rather than silently picking one.

Guard rule names understood by the core:

| Rule | Passes when |
|---|---|
| `no_open_objections` | The thread has no `objection` node with `status = "open"`. |
| `no_open_actions` | The thread has no `action` node with `status = "open"`. |
| `one_human_approval` | At least one non-retracted `approval` node was created by an actor whose ID starts with `human/`. |
| `has_commit_evidence` | The thread has at least one evidence entry with `kind = "commit"`. |

`at_least_one_summary` is not a 3.0 rule because `summary` is not a native node
type.

Operation check sections:

```toml
[[node_rules]]
status = "review"
allowed_types = ["comment", "approval", "objection", "action"]

[revise_rules]
allow_body_revise = ["draft", "open", "working", "review"]
allow_node_revise = ["draft", "open", "working", "review"]

[evidence_rules]
allow_evidence = ["draft", "open", "working", "review", "done", "rejected", "deprecated"]
```

Absent operation-check sections mean no restriction for that operation.

## 4. Storage layout

### 4.1 Authoritative refs

Authoritative thread data:

```text
refs/forum/threads/<thread-id>    # points to a snapshot commit
```

The snapshot commit tree, not the user's working tree, contains the current
thread data. Implementations MUST NOT store authoritative thread data under
tracked paths such as `.forum/threads/<id>/`.

This is the core invariant that preserves branch/worktree independence.

### 4.2 Snapshot tree

Native 3.0 thread ref tip:

```text
thread.toml
body.md
nodes/
  <node-id>.toml
  <node-id>.md
links.toml
evidence.toml
legacy/
  events.ndjson        # migration archive for converted 1.x/2.x threads
```

`thread.toml` is required. `body.md`, `links.toml`, `evidence.toml`, and
`nodes/` MAY be absent when empty. `legacy/events.ndjson` MUST be present on
threads converted from a 1.x/2.x event chain and MUST be absent or ignored for
native 3.0 threads.

Example `thread.toml`:

```toml
schema_version = 3
id = "fg61bcmp"
title = "3.0: Snapshot storage for substantial code reduction"
lifecycle = "proposal"
status = "draft"
tags = ["cross-cutting"]
created_at = "2026-05-02T23:31:40Z"
created_by = "ai/codex"
updated_at = "2026-05-02T23:31:40Z"
updated_by = "ai/codex"
```

Example node metadata:

```toml
id = "n4k9v2mx"
type = "comment"
status = "open"
created_at = "2026-05-03T00:00:00Z"
created_by = "ai/codex"
reply_to = "n1"
```

### 4.3 Repository and local files

Tracked repository configuration lives under `.forum/`:

```text
.forum/
  policy.toml
  actors.toml
  templates/
    thread.md
    proposal.md
    execution.md
    record.md
```

`.forum/` is configuration and templates, not authoritative thread storage.
`policy.toml` defines guards and operation checks. `actors.toml` may define
actor metadata. Templates provide initial body text for new threads.

Local clone state lives under `.git/forum/` and MUST NOT be committed:

```text
.git/forum/
  local.toml
  index.db             # optional cache
  logs/                # optional local diagnostics
```

`local.toml` may contain a default actor and commit identity overrides. Local
state is per clone/worktree and is not authoritative thread storage.

## 5. Read and write protocol

### 5.1 Read path

To read a thread:

1. Resolve `refs/forum/threads/<thread-id>`.
2. Read the tip commit tree.
3. Parse `thread.toml`.
4. Parse optional `body.md`, `nodes/*`, `links.toml`, and `evidence.toml`.
5. Validate schema, state, tag grammar, node references, and link/evidence shape.
6. Return `ThreadSnapshot`.

No event replay is performed for native 3.0 snapshots.

### 5.2 Write path

To write a change:

1. Resolve the current thread ref tip.
2. Read and validate the current snapshot.
3. Apply the requested mutation in memory.
4. Validate the resulting snapshot and policy checks.
5. Write blobs and a new tree.
6. Create a commit whose parent is the previous tip.
7. Update `refs/forum/threads/<thread-id>` with compare-and-swap.

Concurrent writes to the same thread fail at the CAS step. The caller retries
by re-reading the new snapshot.

### 5.3 Commit messages

Snapshot commits SHOULD use concise operation-shaped messages:

```text
[git-forum] thread-create <id>
[git-forum] node-add <id> <node-id>
[git-forum] node-resolve <id> <node-id>
[git-forum] state <id> <from>-><to>
[git-forum] link-add <id> <target> <rel>
```

These messages are display aids for `git forum log`; they are not parsed as
authoritative state.

### 5.4 Log and diff

`git forum log <THREAD>` becomes a Git-history view over the snapshot ref. It
MUST summarize recognized operation-shaped commit messages such as
`thread-create`, `node-add`, `node-resolve`, `state`, and `link-add`. Commits
whose messages are not recognized MUST still be shown as Git commits. The log
command MAY also show changed paths and MAY offer a raw Git commit view, but it
MUST NOT require replaying historical snapshots to compute current state.

`git forum diff <THREAD>` uses Git diffs over snapshot files, especially
`body.md` and `nodes/*.md`.

## 6. Identity scheme

Thread IDs have two forms:

| Form | Example | Use |
|---|---|---|
| Storage form | `fg61bcmp` | Ref names and snapshot fields. |
| Display form | `@fg61bcmp` | Human-facing CLI output. |

The `@` marker is display-only. CLI input MUST accept both `@fg61bcmp` and
`fg61bcmp` wherever a thread ID is expected.

Native 3.0 thread IDs SHOULD be 8-character lowercase base36 tokens. They MUST
be valid as the final path component of `refs/forum/threads/<thread-id>` and
MUST NOT contain `/`, `:`, whitespace, `@{`, `..`, or other characters rejected
by Git ref-name validation.

Implementations MAY accept unique thread ID prefixes for interactive CLI input
when the prefix is at least 4 characters after removing an optional leading
`@`. Ambiguous prefixes MUST fail with candidate IDs.

Node IDs in native 3.0 are opaque path-safe tokens. Implementations SHOULD use
lowercase base36 or hex strings and MUST avoid `/`, `:`, `@`, whitespace, and
Git-ref-problematic characters. Migrated nodes MAY retain legacy event OIDs as
node IDs if they are path-safe.

Actor IDs are claimed identifiers recorded for attribution. Resolution order is:

1. `--as <ACTOR>` command flag;
2. `GIT_FORUM_ACTOR` environment variable;
3. clone-local default actor in `.git/forum/local.toml`;
4. an implementation-derived actor from Git config.

Actor IDs are not authenticated by this specification. Approval nodes record the
actor ID supplied by the command; they do not prove cryptographic sign-off.

## 7. CLI surface

The everyday CLI surface remains recognizable:

```text
git forum new rfc|dec|task|issue|bug <TITLE>
git forum thread new <TITLE> --lifecycle <L> [--tag <T>]...
git forum ls
git forum show <THREAD>
git forum log <THREAD>
git forum diff <THREAD>
git forum comment <THREAD> <BODY>
git forum objection <THREAD> <BODY>
git forum action <THREAD> <BODY>
git forum resolve <THREAD> <NODE>
git forum retract <THREAD> <NODE>
git forum reopen <THREAD> [<NODE>...]
git forum state <THREAD> <STATE>
git forum evidence add <THREAD> --kind <KIND> --ref <REF>
git forum link <FROM> <TO> --rel <REL>
```

Removed from the 3.0 live CLI:

- `claim`, `question`, `summary`, `risk`, and `review` shorthands.
- Legacy node-type creation through `node add --type claim` and similar.
- Any command whose only purpose is repairing event chains (`prune-stale-events`,
  event-specific purge, strict replay repair).

### 7.1 TUI surface

3.0 includes a TUI client. The TUI MUST use the same application command layer
as the CLI for:

- reading thread snapshots;
- validating lifecycle transitions, node mutations, links, evidence, and policy;
- writing snapshot commits;
- handling CAS conflicts.

The TUI MAY provide richer navigation, filtering, editing, and review flows than
the CLI, but it MUST NOT implement a separate storage writer, policy evaluator,
or event-chain compatibility path. TUI behavior for a failed same-thread CAS
write MUST match §10: re-read the latest snapshot and ask the user to re-apply
or retry the intended edit.

## 8. Migration from 1.x/2.x event chains

### 8.1 Strategy

Migration is one-way:

```text
git forum migrate --to 3.0
git forum migrate --to 3.0 --dry-run
```

For each 1.x/2.x `refs/forum/threads/<id>` event-chain ref:

1. Load the 1.x/2.x event chain using the migration tool's replay implementation.
2. Materialize the final `ThreadState`.
3. Convert it to a 3.0 snapshot tree.
4. Create a snapshot commit whose parent is the old event-chain tip.
5. Move `refs/forum/threads/<id>` to the snapshot commit with CAS.
6. Emit a migration report.

Because the snapshot commit keeps the old event-chain tip as parent, the old
events remain reachable through Git history. In addition, migration MUST write
`legacy/events.ndjson` into the snapshot tree so the migrated source events are
inspectable without traversing the old event commits. The 3.0 runtime still
reads only the tip snapshot.

### 8.2 Legacy archive

Migration MUST write:

```text
legacy/events.ndjson
```

This archive is for inspection and export. Core 3.0 commands MUST NOT depend on
it for normal reads or writes. The archive MUST contain the 1.x/2.x source
events in event-chain order as newline-delimited JSON. If a legacy event cannot
be serialized into this archive, migration MUST fail before moving the thread
ref.

### 8.3 Aliases and legacy references

Migration MAY preserve legacy thread IDs through alias refs:

```text
refs/forum/aliases/<legacy-id> -> <thread-id>
```

Alias resolution is read-only. New 3.0 writes always target the canonical
thread ref.

### 8.4 Unmigrated event-chain refs

Except for `git forum migrate --to 3.0`, `git forum migrate --to 3.0 --dry-run`,
and diagnostic commands that explicitly report migration readiness, 3.0 commands
MUST NOT read 1.x/2.x event-chain refs as live threads. When a command encounters
an unmigrated event-chain ref, it MUST fail with a migration-required error that
names the ref and suggests `git forum migrate --to 3.0`.

## 9. Doctor, search, and index

### 9.1 Doctor

`git forum doctor` validates:

- `refs/forum/threads/*` points to readable snapshot commits;
- `thread.toml` schema version and required fields;
- lifecycle/status/tag grammar;
- node metadata/body pairing;
- `reply_to` references target existing nodes;
- link targets are syntactically valid thread IDs, and optionally resolvable;
- policy file shape and selector ambiguity.

Doctor no longer performs strict replay validation for native 3.0 snapshots.

### 9.2 Search and index

A SQLite index MAY exist as an acceleration layer, but it is not part of the
authoritative model. It can be deleted and rebuilt from snapshot refs.

Implementations MAY initially implement search by scanning snapshot trees if
that keeps the core smaller.

## 10. Concurrency and distribution

Within a clone, every write records the current value of
`refs/forum/threads/<thread-id>`, creates a new snapshot commit whose parent is
that value, and updates the ref only if the ref still points at the recorded
old value. This compare-and-swap update prevents silent same-thread lost
updates.

If the ref changed before the update, the write MUST fail without changing the
ref. CAS failures are retryable write conflicts: implementations SHOULD report
that the caller must re-read the latest snapshot and re-apply the intended
change.

Across clones, distribution remains plain Git on `refs/forum/*`. git-forum does
not define a custom push/fetch protocol. Non-fast-forward conflicts are Git
conflicts.

Semantic auto-merge of snapshot commits is deferred. A future RFC may define
field-level merge behavior for disjoint node/link/evidence additions, but 3.0
does not require it. Initial 3.0 behavior MUST NOT silently semantic-merge a
failed CAS update.

## 11. Error handling

New or revised error categories:

| Code | Severity | Trigger |
|---|---|---|
| `SnapshotMissing` | error | Thread ref tip lacks `thread.toml`. |
| `SnapshotSchemaUnsupported` | error | `schema_version` is absent or unsupported. |
| `SnapshotInvalid` | error | Snapshot fields fail schema or grammar checks. |
| `DanglingNodeReference` | error | `reply_to` points to a missing node in the same thread. |
| `AmbiguousPolicyRule` | error | Multiple structured policy rules tie for one operation. |
| `LegacyEventChain` | error | 3.0 command sees an unmigrated 1.x/2.x event chain. |

Event-chain-specific errors such as stale target events are not native 3.0
errors.

## 12. Testing strategy

### Snapshot storage

- Creating a thread writes a valid snapshot commit under `refs/forum/threads/*`.
- Switching branches does not change visible forum threads.
- Linked worktrees sharing a repository see the same forum refs.
- Reading a thread does not replay historical commits.

### Writes

- Node add/edit/resolve/retract/reopen mutate only snapshot files.
- State transitions update `thread.toml` and preserve valid lifecycle/status pairs.
- Links and evidence update their TOML files and remain readable after branch switches.
- Concurrent same-thread writes fail with a retryable CAS error.

### TUI

- TUI thread creation, comment, objection, action, resolve, state, link, and
  evidence flows produce the same snapshot mutations as the corresponding CLI
  operations.
- TUI policy failures and CAS conflicts surface the same error categories as the
  application command layer.
- TUI navigation reads snapshot refs and does not replay 1.x/2.x event chains.

### Migration

- Every migrated 1.x/2.x thread produces a valid 3.0 snapshot.
- Old event commits remain reachable as parents of the migration snapshot commit.
- Every migrated 1.x/2.x thread contains `legacy/events.ndjson`.
- Unmigrated 1.x/2.x event-chain refs are rejected by normal 3.0 read and write
  commands with a migration-required error.
- Migrated legacy node labels do not affect live node-type behavior.
- Dry-run reports every planned ref update without writing objects.

### Policy

- Structured selectors match lifecycle and tags without invoking a string expression parser.
- Ambiguous operation rules fail loudly.
- Removed legacy policy forms are rejected with actionable migration hints.

## 13. Non-goals

- Storing authoritative thread data in tracked working-tree paths.
- Preserving domain-event replay as the normal read model.
- Supporting unmigrated 1.x/2.x event-chain refs as read-only live threads.
- Preserving deprecated node shorthands in 3.0.
- Implementing custom cross-clone distribution or merge protocol.
- Requiring SQLite for correctness.
- Requiring a GitHub bridge as part of the 3.0 core.

## Appendix A: Expected simplification

The snapshot-ref model is expected to shrink or remove these implementation
surfaces:

| Surface | 3.0 impact |
|---|---|
| Event enum/builders/read-write path | Mostly removed from runtime. |
| Replay state machine | Replaced by direct snapshot parsing. |
| Strict replay diagnostics | Replaced by snapshot schema validation. |
| Stale-event pruning/repair | Removed for native 3.0 snapshots. |
| Domain timeline rendering | Replaced by Git-history view. |
| Event-specific purge | Replaced by Git history rewrite guidance or whole-snapshot redaction. |
| Index coherence | Optional cache only. |
| Legacy node/state/kind compatibility | Migration-only. |

This does not by itself remove every large feature, but it removes the central
event-sourcing complexity while preserving the `refs/forum/*` namespace that
makes forum state branch-independent.
