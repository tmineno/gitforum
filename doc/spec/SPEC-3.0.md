# git-forum Product Specification - 3.0

Version 3.0 - 2026-05-06
Status: **Authoritative**. This document is self-contained: every
normative 3.0 rule is written here directly rather than inherited by reference
from an earlier specification.
Discussion thread: `fg61bcmp`.
Bound by `doc/spec/CORE-VALUE.md` - when this document conflicts with the core
value statement, this document is wrong and must be revised.

> 3.0 uses a `thread` model classified by a policy-controlled `category`, with
> four canonical node types: `comment`, `approval`, `objection`, and `action`.
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
| Compatibility | Legacy event/state/node compatibility in runtime | Lossy one-shot migration; legacy events are archival, not the live model |
| Policy classification | Facet expression strings over kinds, states, and tags | Required category registry; tags are not policy enforcement keys |

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
| `id` | string | Thread ID. |
| `title` | string | Human-readable title. |
| `category` | string | Required policy category from §2.4. |
| `status` | string | Current state allowed by the category registry in §3.1. |
| `tags` | array | Free-form labels using the grammar in §2.4. |
| `created_at` | datetime | Creation timestamp. |
| `created_by` | string | Actor ID. |
| `updated_at` | datetime | Timestamp of last snapshot update. |
| `updated_by` | string | Actor ID for last snapshot update. |

Optional fields:

| Field | Type | Description |
|---|---|---|
| `branch` | string | Branch scope advisory. |
| `supersedes` | array | Convenience summary of outgoing `supersedes` links, if implementations choose to denormalize. |
| `visibility` | string | `"public"` or `"private"`. Controls whether the thread is materialized into the published namespace by `git forum push`. **Absent means `private`.** Older writers that drop unknown keys under-publish (recoverable) instead of leaking (unrecoverable); the asymmetry of failure modes locks the absent-is-private rule in. No `schema_version` bump is required. |

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

### 2.4 Category and tag vocabulary

`category` is the required classification facet. It selects the policy category
that defines a thread's initial status, valid states, valid transitions,
transition guards, and operation checks.

Categories are policy-controlled and MUST satisfy the same grammar as tags.
Native 3.0 implementations MUST always provide these built-in categories:

| Category | Meaning |
|---|---|
| `rfc` | Proposal-style discussion that starts in `draft` and is accepted or rejected after review. |
| `task` | Work tracking that starts in `open` and may move through `working` and `review`. Defect reports and decision records are also `task` threads, distinguished by a `bug` or `decision` tag. |

Repositories MAY define additional categories in `.forum/policy.toml`.
Repositories MAY also override built-in category definitions category-by-category.
An override affects the category's policy definition, but it does not remove the
category name from the built-in set.

`tags` are free-form labels for filtering, search, and display. Tags MUST NOT be
used as policy enforcement selectors in 3.0 core behavior.

Every category and tag MUST satisfy:

- ASCII lowercase only: `[a-z0-9-]`.
- Starts with a lowercase letter: `[a-z]`.
- Length is 2 to 32 characters.
- Not equal to `all`, `none`, `any`, or `untagged`.
- Contains no spaces, slashes, colons, `@`, or `!`.

### 2.5 Connection to code

Threads stay connected to the repository content they discuss through three
mechanisms. None of them introduce cross-thread enforcement (per CORE-VALUE:
Bounded Policy).

1. **Evidence records.** `evidence.toml` (§2.3) lets a thread point at a
   commit, file, hunk, test, benchmark, doc, or external reference. Evidence
   is the primary linkage from a thread to the working tree.
2. **Branch binding.** The optional `branch` field on `thread.toml` (§2.1)
   records the Git branch a thread concerns. It is advisory: it does not gate
   any operation. Readers and listing commands MAY use it to surface threads
   relevant to the current branch. 3.0 does not define `<THREAD>`-argument
   defaulting from the bound branch; every CLI invocation that takes a
   `<THREAD>` argument requires it explicitly.
3. **Commit-message validation.** An optional `commit-msg` Git hook installed
   by git-forum validates thread IDs that appear on a `Refs:` trailer line in
   a commit message. The trailer takes the form `Refs: <id>[, <id>...]`,
   where each `<id>` is a bare 3.0 thread ID (no `@` prefix; §6). The hook
   MUST NOT scan the commit message body for bare IDs — base36 thread tokens
   collide with abbreviated commit hashes and free-form prose. The hook MUST
   NOT block commits that have no `Refs:` trailer. It MUST fail commits
   whose `Refs:` trailer names an undefined thread ID.

These mechanisms are required surface (per CORE-VALUE: Code-Adjacent
Deliberation). Their CLI entry points are listed in §7.

## 3. Category registry and policy

### 3.1 Category registry

Policy enforcement is keyed by `category`. 3.0 does not define a selector
language over tags or other facets.

Each category definition MUST provide:

| Field | Type | Meaning |
|---|---|---|
| `initial_status` | string | Status assigned to new threads in this category. |
| `statuses` | array | Complete set of valid statuses for this category. |
| `transitions` | array | Valid `from->to` status transitions for this category. |

Example category registry:

```toml
[categories.rfc]
initial_status = "draft"
statuses = ["draft", "open", "review", "done", "rejected", "withdrawn", "deprecated"]
transitions = [
  "draft->open",
  "draft->withdrawn",
  "open->review",
  "open->rejected",
  "open->withdrawn",
  "review->done",
  "review->rejected",
  "done->open",
  "rejected->open",
  "done->deprecated",
  "rejected->deprecated",
]

[categories.task]
initial_status = "open"
statuses = ["open", "working", "review", "done", "rejected", "deprecated"]
transitions = [
  "open->working",
  "open->review",
  "open->done",
  "open->rejected",
  "working->review",
  "working->done",
  "working->rejected",
  "review->done",
  "review->working",
  "review->rejected",
  "done->open",
  "rejected->open",
  "done->deprecated",
  "rejected->deprecated",
]
```

`done->open` and `rejected->open` are the reopen edges that back the
everyday `git forum reopen <ID>` shorthand. They are part of the built-in
defaults; repos that prefer one-way close semantics MAY override either
category to drop them.

A transition is valid only when it is listed in the thread category's
`transitions` array. A status is valid only when it is listed in the thread
category's `statuses` array. New threads MUST start at the category's
`initial_status`.

State transitions update `thread.toml` directly. The historical record of a
transition is the Git commit that changed `status`, plus the commit message and
diff. There is no separate `state` event object in the live model.

### 3.2 Transition guards

Transition guards are attached directly to a category transition:

```toml
[categories.rfc.guards]
"review->done" = ["one_approval", "no_open_objections"]
```

Guard rule names understood by the core:

| Rule | Passes when |
|---|---|
| `no_open_objections` | The thread has no `objection` node with `status = "open"`. |
| `no_open_actions` | The thread has no `action` node with `status = "open"`. |
| `one_approval` | At least one non-retracted `approval` node exists on the thread, regardless of actor type. |
| `has_commit_evidence` | The thread has at least one evidence entry with `kind = "commit"`. |

`at_least_one_summary` is not a 3.0 rule because `summary` is not a native node
type.

### 3.3 Operation checks

Operation checks are category-scoped:

```toml
[categories.rfc.creation]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[categories.rfc.allowed_node_types]
review = ["comment", "approval", "objection", "action"]

[categories.rfc.revise]
allow_body_revise = ["draft", "open", "review"]
allow_node_revise = ["draft", "open", "review"]

[categories.rfc.evidence]
allow_evidence = ["draft", "open", "working", "review", "done", "rejected", "deprecated"]
```

Absent operation-check sections mean no restriction for that operation. A
category definition with an unknown guard name, unknown status, duplicate
transition, or transition that references a status outside `statuses` is invalid.

## 4. Storage layout

### 4.1 Authoritative refs

Authoritative thread data:

```text
refs/forum/threads/<thread-id>      # points to a snapshot commit
refs/forum/published/<thread-id>    # derived, public-only mirror
```

`refs/forum/threads/<thread-id>` is the authoritative ref: full content,
private and public alike. The snapshot commit tree, not the user's
working tree, contains the current thread data. Implementations MUST
NOT store authoritative thread data under tracked paths such as
`.forum/threads/<id>/`.

`refs/forum/published/<thread-id>` is a derived, public-only mirror
maintained by `git forum push` (§5.5). It holds a parentless,
force-updated snapshot of threads with `visibility = "public"`,
filtered through the exclusion pipeline in §5.5. Authoritative refs
MUST NOT be pushed to a public-consumer remote; the published
namespace is the only namespace `git forum push` writes to a remote.

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
`nodes/` MAY be absent when empty. `legacy/events.ndjson` SHOULD be present by
default on threads converted from a 1.x/2.x event chain when source events are
available. Core 3.0 reads MUST ignore it.

Example `thread.toml`:

```toml
schema_version = 3
id = "fg61bcmp"
title = "3.0: Snapshot storage for substantial code reduction"
category = "rfc"
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
    rfc.md
    task.md
```

`.forum/` is configuration and templates, not authoritative thread storage.
`policy.toml` defines category registry overrides, guards, and operation checks.
`actors.toml` may define actor metadata. Templates provide initial body text for
new threads by category.

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

1. Resolve `refs/forum/threads/<thread-id>`. If absent, fall back to
   `refs/forum/published/<thread-id>` (§4.1). If both are absent, the
   thread does not exist.
2. Read the tip commit tree.
3. Parse `thread.toml`.
4. Parse optional `body.md`, `nodes/*`, `links.toml`, and `evidence.toml`.
5. Validate schema, category/status/tag grammar, node references, and link/evidence shape.
6. Return `ThreadSnapshot`.

When both refs resolve to the same `<id>`, the authoritative ref wins.
Listing commands MUST deduplicate by ID across both namespaces. The
mixed case is normal on trusted-collaborator clones; the published-only
case is normal on public-consumer clones (§5.4).

No event replay is performed for native 3.0 snapshots.

### 5.2 Write path

To write a change:

1. Resolve the current thread ref tip.
2. Read and validate the current snapshot.
3. Apply the requested mutation in memory.
4. Validate the resulting snapshot and category policy checks.
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

### 5.5 Publish protocol

`git forum push` projects threads with `visibility = "public"`
(§2.1) into `refs/forum/published/<thread-id>` (§4.1) and
propagates the resulting refs to a remote. Authoritative refs MUST
NOT be transmitted by this command.

For each public thread the publisher MUST:

1. **Filter structured references.**
   - Drop entries from `links.toml` whose target is a non-public
     thread. Substituted placeholders MUST NOT be written; a
     placeholder leaks the existence, count, or topology of
     private threads.
   - Drop entries from `evidence.toml` where `kind = "thread"` and
     the target is non-public.
2. **Pass body and node text through unchanged.** `body.md`,
   `nodes/<id>.md`, and `nodes/<id>.toml` MUST NOT be rewritten,
   redacted, or otherwise mutated by the publisher. Authors are
   responsible for the contents of public bodies. The publisher's
   privacy contract covers structured references only.
3. **Run a pre-publish lint.** Scan `body.md` and node body text
   for tokens that name a thread the local index marks as
   private. The lint MUST recognize the `@<id>` display form, the
   full `refs/forum/threads/<id>` ref form, labeled-context bare
   IDs after `Refs:`, `thread:`, `parent:`, or `reply_to:`
   markers, and bare 8-char tokens that exact-match a known
   private thread ID. Bare tokens that do not exact-match a known
   private ID MUST NOT warn (so abbreviated commit hashes and
   base36 nonces do not produce false positives). Structured
   references in `links.toml` and `evidence.toml` are handled by
   the filter step in (1)–(2), not by the lint. The lint is
   informational by default and MUST exit non-zero only under
   `--strict`.
4. **Build a parentless snapshot commit.** The published commit
   MUST be parentless (`git commit-tree <tree-sha>` with no
   `-p`). `git log refs/forum/published/<id>` therefore shows
   only the current snapshot; there is no walk into prior
   snapshots. Author, committer, dates, and signing follow the
   operator's normal Git config; the publisher MUST NOT invent a
   synthetic identity or pin timestamps.
5. **Idempotency by tree.** Before writing, the publisher MUST
   compare the recomputed published *tree* SHA against the tree
   pointed at by the current `refs/forum/published/<id>`. When
   trees match, the publisher MUST skip that thread (no new
   commit, no ref update, no remote transmission). Tree
   equivalence — not commit-SHA reproducibility — is the
   property the publisher relies on, because commit metadata is
   operator-local.
6. **Force-update the published ref.** On a content change the
   published ref is force-updated to the new parentless commit;
   the prior commit becomes unreachable from any ref. The
   published fetch refspec uses `+refs/forum/published/*:...` so
   consumers accept the force-update.

### 5.6 Withdrawal protocol

A thread becomes withdrawn from the published namespace when:

- a `private → public` toggle has been reversed via
  `git forum thread set-visibility <id> private`,
- the authoritative thread has been deleted entirely, or
- a `refs/forum/published/<id>` exists locally with no
  corresponding authoritative thread.

`git forum push` is the operation that propagates withdrawals to
the remote. The flow MUST be:

1. **Identify withdrawal candidates.** Any `<id>` where
   `refs/forum/published/<id>` exists locally and the
   authoritative thread is either absent or has
   `visibility = "private"`.
2. **Stage remote deletions.** Build the push refspec list with
   `:refs/forum/published/<id>` for each candidate, alongside the
   normal `+refs/forum/published/<id>:refs/forum/published/<id>`
   updates for creates/updates.
3. **Push creates, updates, and deletions in a single
   invocation.** Per-ref atomicity is whatever the underlying
   `git push` already provides; this specification does not
   introduce a new transaction layer.
4. **Update local refs only on remote success.** For every
   withdrawal the remote accepted, the publisher MUST delete the
   local `refs/forum/published/<id>`. For every withdrawal the
   remote rejected (permission denied, ref protection, transient
   network failure), the local published ref MUST be preserved
   so a retry of `git forum push` reattempts the deletion
   without manual cleanup. Implementations MUST NOT delete the
   local ref before the remote operation succeeds (the
   "preserve-then-retry" rule).
5. **Report outcomes.** The summary MUST count successful
   withdrawals (local + remote both gone) separately from
   failures (remote refused, local preserved). The exit code
   MUST be non-zero whenever any remote operation failed,
   regardless of `--strict`.

### 5.7 Privacy contract and residual retention

`git forum push` commits to:

- not transmitting private thread bodies,
- not transmitting structured references (links, evidence) from
  public threads to private threads.

It does NOT commit to:

- redacting private thread IDs that appear in public body or
  node text,
- preventing observers from inferring the existence or
  correlation of private threads via text references,
- catching every textual reference at lint time (the lint is
  best-effort author tooling, not a security boundary),
- recalling content from clients that have already fetched a
  prior published snapshot,
- bounding the GC schedule of the canonical remote or of forks
  and mirrors.

Withdrawal removes a ref pointer but does not retract objects
that consumers have already fetched. Operators MUST NOT treat
withdrawal as content recall. Repositories that need stricter
guarantees should rely on transport-layer access control rather
than on the published namespace.

## 6. Identity scheme

Thread IDs have one form:

| Form | Example | Use |
|---|---|---|
| Thread ID | `fg61bcmp` | Ref names, snapshot fields, CLI input, and CLI output. |

3.0 does not define a display marker for thread IDs. Human-facing output MUST
show bare thread IDs such as `fg61bcmp`. CLI input MUST treat `@fg61bcmp` as an
invalid 3.0 thread ID input.

Native 3.0 thread IDs SHOULD be 8-character lowercase base36 tokens. They MUST
be valid as the final path component of `refs/forum/threads/<thread-id>` and
MUST NOT contain `/`, `:`, `@`, whitespace, `@{`, `..`, or other characters
rejected by Git ref-name validation.

Implementations MAY accept unique thread ID prefixes for interactive CLI input
when the prefix is at least 4 characters. Ambiguous prefixes MUST fail with
candidate IDs.

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
git forum new <CATEGORY> <TITLE> [--tag <T>]...
git forum thread new <TITLE> --category <CATEGORY> [--tag <T>]...
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
git forum branch bind <THREAD> [<BRANCH>]
git forum branch unbind <THREAD>
git forum hooks install
git forum init [--public-only] [--auto-push]
git forum thread set-visibility <THREAD> public|private [--force]
git forum push [<REMOTE>] [--strict]
```

The `branch ...` and `hooks install` commands implement the
connection-to-code mechanisms from §2.5. `branch bind` sets the
optional `branch` field on `thread.toml` to the named branch,
defaulting to the current branch when `<BRANCH>` is omitted.
`branch unbind` clears the field. `hooks install` installs the
optional `commit-msg` validator hook into the local clone.

`init`, `thread set-visibility`, and `push` implement the publish
protocol (§5.5–§5.7):

- `init` configures the per-remote fetch refspec. The default is
  trusted-collaborator mode:
  `+refs/forum/threads/*:refs/forum/threads/*` plus
  `+refs/forum/published/*:refs/forum/published/*`. With
  `--public-only`, only the published refspec is configured —
  authoritative refs are never imported on these clones.
  `--auto-push` additionally sets
  `remote.<name>.push = +refs/forum/published/*:refs/forum/published/*`
  on every remote, so a bare `git push` propagates the published
  namespace. `init` MUST NOT configure
  `refs/forum/threads/*` as a push refspec.
- `thread set-visibility` toggles the `visibility` field on
  `thread.toml`. The `private → public` transition is the
  explicit allowlist step; there is no "publish all" flag. The
  `public → private` transition warns once per session about
  irrevocability and requires `--force` in non-interactive runs.
  The flip is recorded immediately on the authoritative thread;
  `refs/forum/published/<id>` is removed on the next
  `git forum push` per §5.6.
- `push` runs the publish protocol of §5.5 followed by the
  withdrawal protocol of §5.6. The remote argument defaults to
  `origin`. `--strict` exits non-zero on any pre-publish lint
  warning; failed remote operations always exit non-zero.

Removed from the 3.0 live CLI:

- `claim`, `question`, `summary`, `risk`, and `review` shorthands.
- Legacy node-type creation through `node add --type claim` and similar.
- Any command whose only purpose is repairing event chains (`prune-stale-events`,
  event-specific purge, strict replay repair).

### 7.1 TUI surface

3.0 includes a TUI client. The TUI MUST use the same application command layer
as the CLI for:

- reading thread snapshots;
- validating category transitions, node mutations, links, evidence, and policy;
- writing snapshot commits;
- handling CAS conflicts.

The TUI MAY provide richer navigation, filtering, editing, and review flows than
the CLI, but it MUST NOT implement a separate storage writer, policy evaluator,
or event-chain compatibility path. TUI behavior for a failed same-thread CAS
write MUST match §10: re-read the latest snapshot and ask the user to re-apply
or retry the intended edit.

## 8. Migration from 1.x/2.x event chains

### 8.1 Strategy

Migration is one-way and intentionally lossy. The migration tool preserves the
minimum useful user-facing material:

- thread title and body;
- readable discussion content;
- outgoing links;
- tags;
- legacy kind or lifecycle mapped to a 3.0 category;
- the legacy final status, when it is a valid status under the target category.

It does not preserve exact 1.x/2.x state-machine semantics, policy outcomes,
node-type behavior, evidence behavior, repair metadata, or timeline semantics.
Status preservation is best-effort: the legacy final status is folded onto the
canonical 2.0 status name, then carried over only if the target category's
`statuses` list (§3.1) accepts it. Otherwise the snapshot uses the target
category's `initial_status` and the reset is recorded as a `state` omission in
the migration report.
Implementations MAY use the existing 1.x/2.x replay/materialization code inside
the migration module to avoid duplicating event parsing logic, but that legacy
adapter MUST NOT be part of the native 3.0 read or write path.

Commands:

```text
git forum migrate --to 3.0
git forum migrate --to 3.0 --dry-run
```

For each 1.x/2.x `refs/forum/threads/<id>` event-chain ref:

1. Materialize the legacy thread through a migration-only 1.x/2.x adapter.
2. Map the legacy kind or lifecycle to a 3.0 category using the fixed table in
   §8.3.
3. Project only the preserved material listed above into a 3.0 snapshot.
4. Set the snapshot status to the legacy final status when that status (folded
   to its canonical 2.0 name) appears in the target category's `statuses`
   list. Otherwise use the target category's `initial_status` and record the
   reset as a `state` omission in the migration report.
5. Convert preserved textual discussion material to `body.md` and `comment`
   nodes. Legacy approvals, objections, actions, reviews, questions, claims,
   risks, summaries, and other non-native semantics MAY be flattened to
   `comment` nodes.
6. Convert representable outgoing links to `links.toml`. Invalid or
   unrepresentable links MAY be omitted and MUST be listed in the migration
   report.
7. Preserve valid tags. Invalid tags MAY be omitted and MUST be listed in the
   migration report.
8. Create a snapshot commit whose parent is the old event-chain tip.
9. Move `refs/forum/threads/<id>` to the snapshot commit with CAS.
10. Emit a migration report.

Because the snapshot commit keeps the old event-chain tip as parent, the old
events remain reachable through Git history. The 3.0 runtime still reads only
the tip snapshot.

### 8.2 Legacy archive

Migration SHOULD write by default:

```text
legacy/events.ndjson
```

This archive is for inspection and export. Core 3.0 commands MUST NOT depend on
it for normal reads or writes. The archive SHOULD contain the 1.x/2.x source
events in event-chain order as newline-delimited JSON when those source events
are available. Writing the archive MUST NOT require semantic reconstruction of
old state. If archive generation is omitted or incomplete, migration MUST list
that fact in the migration report.

### 8.3 Category mapping

Migration uses a fixed built-in mapping from 1.x/2.x thread kind or lifecycle to
3.0 category:

| 1.x/2.x source | 3.0 category |
|---|---|
| `rfc` | `rfc` |
| `dec` | `task` |
| `task` | `task` |
| `issue` | `task` |
| `bug` | `task` |
| `proposal` | `rfc` |
| `execution` | `task` |
| `record` | `task` |
| unrecognized | `task` |

Migration MUST NOT require repository-specific category mapping logic. To
preserve the defect/decision classifications that the category collapse
removes, migration MUST also add the following canonical 3.0 tags:

| 1.x/2.x source | Added 3.0 tag |
|---|---|
| `bug`, `issue` | `bug` |
| `dec`, `record` | `decision` |

The source kind or lifecycle MAY additionally be preserved verbatim as a tag
when it satisfies the grammar in §2.4 and differs from the canonical tag
above.

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
- category/status/tag grammar;
- category registry shape and transition references;
- node metadata/body pairing;
- `reply_to` references target existing nodes;
- link targets are syntactically valid thread IDs, and optionally resolvable;
- policy file shape;
- the publish namespace is consistent with thread visibility.

Publish-namespace advisories cross-check `refs/forum/threads/*` and
`refs/forum/published/*` (§4.1):

- `auth-without-published` (INFO) — a thread has
  `visibility = "public"` but no `refs/forum/published/<id>`.
  Likely "you forgot to push," but not publishing is a legitimate
  state, so this is informational rather than a warning.
- `visibility-mismatch` (WARN) — both refs exist and the published
  tree's `thread.toml` disagrees with the authoritative
  visibility. Would only happen across writer-version skews.
- `stale-published` (WARN) — `refs/forum/published/<id>` exists
  locally with no authoritative thread, or the authoritative
  thread is `visibility = "private"`. Indicates an interrupted
  withdrawal (§5.6); re-run `git forum push` to retry the remote
  delete.

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
| `CategoryUnknown` | error | Thread category is not defined by built-in or repository policy. |
| `CategoryPolicyInvalid` | error | Category policy references unknown statuses, unknown guards, or malformed transitions. |
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
- State transitions update `thread.toml` and preserve valid category/status pairs.
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
- Every migrated 1.x/2.x thread preserves title/body discussion content, valid
  links, valid tags, and a 3.0 category.
- Migrated threads use the target category's `initial_status` rather than
  preserving legacy status semantics.
- Old event commits remain reachable as parents of the migration snapshot commit.
- `legacy/events.ndjson` is written by default when source events are available,
  and normal 3.0 reads do not depend on it.
- Unmigrated 1.x/2.x event-chain refs are rejected by normal 3.0 read and write
  commands with a migration-required error.
- Omitted invalid links, omitted invalid tags, and incomplete legacy archives
  are listed in the migration report.
- Dry-run reports every planned ref update without writing objects.

### Policy

- Category policy validates initial statuses, status sets, transitions, guards,
  and operation checks without invoking a string expression parser.
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
