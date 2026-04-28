# git-forum Product Specification — 2.0 (DRAFT)

Version 2.0-draft — 2026-04-28
Status: **DRAFT for discussion**. Not yet authoritative. Inherits from SPEC.md v1.2 except where
explicitly overridden below.

> This draft introduces two structural changes to the 1.x model:
> 1. **Kind reduction** — the four thread kinds (`rfc`, `dec`, `task`, `issue`) collapse into a
>    single `thread` entity carried by `lifecycle` + free-form `tags`. The four 1.x kinds remain
>    as **stable CLI presets** (`new rfc`, `new task`, `new bug`, `new dec`) — the muscle memory
>    is preserved indefinitely; only the underlying schema changes.
> 2. **Topic as named context** — a new `topic` entity provides a memorable handle for
>    grouping related threads. **Threads remain the primary unit of work**; topics are
>    optional context wrappers, not a required ceremony layer. Standalone threads (no topic)
>    are first-class throughout the CLI, TUI, and default views.
>
> The topic concept in 2.0 is intentionally **slim**: a named container with an optional
> charter, and an archive flag. There is no topic state machine, no topic-level guards, and
> no nesting in 2.0. These capabilities are explicitly deferred to future minor releases (see
> Appendix A.3 for the forward-compatibility plan).
>
> The motivating analysis is recorded separately in ADR-002 (kind reduction), ADR-003 (topic
> handles), ADR-004 (migration), and ADR-005 (cross-clone conflict resolution). This document
> specifies the resulting model.

## 1. Overview

### 1.1 What changes versus 1.x

| Concern | 1.x | 2.0 |
|---|---|---|
| Primary unit of work | Thread (`RFC-...`, `JOB-...`) | **Thread** (unchanged). Topic is a named context that can group threads, but threads stay first-class. |
| Thread classification | `kind` enum: `rfc` / `dec` / `task` / `issue` | **Single required facet** (`lifecycle`) + free-form `tags` |
| State machines | 4 kind-specific machines | 1 unified machine, allowed states gated by `lifecycle` facet |
| Top-level CLI | `git forum new rfc ...` etc. | `git forum new rfc/task/bug/dec ...` remain as the **stable everyday surface**; `git forum thread new --lifecycle ...` is the canonical/scriptable form |
| Topic concept | None (links only) | Named context with handle and (optional) child threads. Slim by design: **no state machine, no guards, no nesting in 2.0**. |
| Thread ID readability | Opaque (`RFC-6m4kap23`) — unmemorable | Unchanged. Topics carry the readable handle. |

### 1.2 Design principles (additions to 1.x)

In addition to the six principles in SPEC.md §1.1, 2.0 adds:

7. **Composable taxonomy.** Thread classification is built from independent facets, not enumerated
   kinds. New use cases extend the facet vocabulary, not the kind set.
8. **Two-layer identity.** Receipt-quality identity (opaque, conflict-free) is decoupled from
   handle-quality identity (slug, memorable, possibly mutable). Threads use the former, topics
   use the latter.
9. **Topic as named context, not as ceremony.** Topics in 2.0 are optional grouping
   devices with memorable names. They do not enforce sequencing, do not run state machines, and
   do not gate transitions. Threads — including standalone threads — remain the primary unit of
   capture and work; topics wrap them when grouping helps. Coordination semantics — when
   warranted — are added later as opt-in extensions (see Appendix A.3).
10. **Quick-capture-first.** A short bug report or note must take seconds, not minutes. Stable
    kind presets (`new bug`, `new task`, `new rfc`, `new dec`) and the standalone-thread default
    keep the friction low for common cases. Topic attachment, charters, and structured
    proposals are available when warranted, never required.

### 1.3 Implementation constraints

Unchanged from SPEC.md §1.2.

## 2. Core model

### 2.1 Topic (NEW)

A **topic** is a long-lived container for related threads. It represents a unit of work a human
or team can name, point at, and reason about as a whole (e.g., "the payment rewrite", "Q2
onboarding").

Required fields:

| Field | Type | Description |
|---|---|---|
| `handle` | string | Human-readable handle (e.g. `!payment-rewrite`) — unique within the repo |
| `id` | string | Internal opaque ID (e.g. `x8n2q1d4`, 8 base36 chars) — used for ref storage and aliasing. Topic IDs have no prefix in storage; the `!` symbol appears only in the user-facing handle layer. |
| `title` | string | Human-readable title |
| `created_at` | datetime | Creation timestamp |
| `created_by` | string | Actor ID |

Optional fields:

| Field | Type | Description |
|---|---|---|
| `body` | string | Topic charter / brief |
| `aliases[]` | array | Additional handles that resolve to this topic |
| `archived_at` | datetime | Archive timestamp (set = archived; unset = active) |

A topic has **no `status` enum and no state machine** in 2.0. The only lifecycle binary is
`archived_at` (present or absent). This is a deliberate scope choice — see Appendix A.3 / F-W1
for the path to add a richer state model later.

#### 2.1.1 Topic handle

- **Display format**: `!<slug>` where slug is `[a-z0-9-]+`, length 3–48. The `!` is the topic
  type marker; the slug is what is stored under `refs/forum/aliases/<slug>`.
- **Allocation**: derived from title via slug + collision-resolved petname suffix
  (see §6.1).
- **Mutability**: handles MAY be renamed. Old handles become aliases automatically. Aliases never
  expire (links keep working).

#### 2.1.2 Topic membership

A thread belongs to **at most one** topic. Membership is recorded via a `topic_attach` event
on the thread. A topic may contain zero or more threads.

Standalone threads (no topic) are permitted (see §2.2.4).

#### 2.1.3 Topic handle in references

Within a topic context, child threads may be referenced by **short index**:

```
!payment-rewrite          # the topic itself
!payment-rewrite/3        # the 3rd attached thread (display order)
```

The `/N` short index is a **display-only convenience**, not an identifier:

- It is computed from locally-visible `topic_attach` events ordered by
  `(timestamp, actor_id, event_oid)` — see §8.3 for cross-clone behaviour.
- It MAY appear as input to interactive CLI commands (e.g.
  `git forum show !payment-rewrite/3`) and in `show` / `ls` output for human convenience.
- It is **rejected** anywhere a value would be persisted: as an evidence ref, link target,
  topic attach argument, or in a commit message scanned by the `commit-msg` hook (§13
  `ShortIndexInPersistedRef` is an error in 2.0, not a warning).

Named role labels (e.g. `!foo/design`) are **not specified in 2.0**. Forward-compatible: if
roles are introduced later, the syntax is reserved.

### 2.2 Thread

A **thread** is an append-only event chain representing a single, focused contribution to a body of
work (a question, a proposal, an implementation task, a recorded decision, etc.).

Required fields:

| Field | Type | Description |
|---|---|---|
| `id` | string | Opaque content-addressed ID. **Display form**: `@XXXXXXXX` (8 base36 chars). **Storage form**: bare `XXXXXXXX` under `refs/forum/threads/`. See §6.2. |
| `title` | string | Human-readable title |
| `status` | enum | Current state (see §3.2) |
| `facets` | object | See §2.3 |
| `created_at` | datetime | Creation timestamp |
| `created_by` | string | Actor ID |

Optional fields:

| Field | Type | Description |
|---|---|---|
| `body` | string | Thread body |
| `topic` | string | Owning topic handle (absent for standalone threads) |
| `scope.branch` | string | Bound Git branch |
| `links[]` | array | Thread-to-thread links |

#### 2.2.1 ID prefix change

Thread IDs in 2.0 use the unified prefix `t-` (no kind embedded in the ID). Kind information moves
to `facets.lifecycle` and conventional `tags` (e.g. `bug`, `task`, `cross-cutting`).

Legacy 1.x IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, `JOB-...`, `DEC-...`) remain valid for reading and
referencing in repos that have been migrated. New thread allocation always uses `t-`.

#### 2.2.2 Standalone threads

Threads MAY exist without a topic. This is the natural form for:

- Bug reports captured quickly before triage
- Questions that don't yet belong to any workstream
- One-off observations

Standalone threads can be promoted into a topic at any time via `topic attach`. A standalone
thread without an `attach` event after some configurable inactivity is reported by `doctor` as
"orphan" — informational, not blocking.

### 2.3 Facets

A thread's classification is **one required facet** plus free-form tags.

#### 2.3.1 Required facet

| Facet | Values | Meaning |
|---|---|---|
| `lifecycle` | `proposal` / `execution` / `record` | How the thread progresses (gates the state machine) |

`lifecycle` is the only required facet because it is the only one the **state machine itself**
depends on (§3.2.1). Everything else — bug-vs-task, cross-cutting-vs-local, sub-team routing — is a
tag.

Earlier drafts of 2.0 included additional required facets (`intent` with 5 values; `scope` with
`cross-cutting` / `local`). Both were removed during scoping — see §2.3.4 for rationale.

#### 2.3.2 First-class tags

Threads carry a free-form `tags[]` (string array). Tags are first-class:

- Queryable in `ls` and search.
- Referenceable in policy (`creation_rules.execution.tag.task`, `guards` with tag predicates,
  etc.).
- The discriminator for sub-categories within a lifecycle (e.g. `bug` vs `task` within
  `lifecycle=execution`).

Three conventional tags are pre-installed by `git forum init` and used by the kind presets
(§9.2):

| Tag | Conventional meaning |
|---|---|
| `bug` | Observation-style execution thread (legacy `ISSUE` / `ASK`) |
| `task` | Work-style execution thread (legacy `TASK` / `JOB`) |
| `cross-cutting` | Wide-impact thread (legacy `RFC` carries this by convention) |

Repos may add or remove conventional tags freely. Nothing in the core model depends on these
specific values.

#### 2.3.3 Mapping from 1.x kinds

The 1.x four-kind taxonomy maps to 2.0 as follows:

| 1.x kind | lifecycle | conventional tags |
|---|---|---|
| `rfc` | `proposal` | `cross-cutting` |
| `dec` | `record` | (none) |
| `task` (`JOB`) | `execution` | `task` |
| `issue` (`ASK`) | `execution` | `bug` |

These four combinations are exposed as **kind presets** (compatibility shorthands; §9.2).

#### 2.3.4 Why one required facet and not more

`intent` (5 values) was rejected for these reasons:

- `decision` — **zero** usage in 1.x dogfood (DEC kind unused). Recording a decision belongs at
  the node level (`summary` node) inside whatever thread reached that decision.
- `question` — questions are predominantly node-level inside other threads.
- `observation` / `work` / `claim` — these describe *body framing*, not *progression-shape*. Tags
  cover framing without forcing premature classification.

`scope` (cross-cutting vs local) was rejected because:

- Only one of the four 1.x kinds (`rfc`) used `cross-cutting`; the other three were always
  `local`. The facet carried 1 bit of information meaningful only for proposals.
- The rfc-vs-dec distinction is already made by `lifecycle` (`proposal` vs `record`); `scope`
  added no orthogonal information for that axis.
- Proposals that *aren't* repo-wide (rare) can be expressed as `lifecycle=proposal` without the
  `cross-cutting` tag.
- Policies that need to distinguish wide-impact work can predicate on `tag=cross-cutting`.

`lifecycle` survives as the sole required facet because the state machine literally cannot work
without knowing which state set applies. Everything else is a tag. This is the floor.

### 2.4 Event

Unchanged from SPEC.md §2.3. New event types added in 2.0:

| Event type | Purpose |
|---|---|
| `topic_create` | Initialize a topic (recorded on topic ref) |
| `topic_archive` | Mark topic as archived (sets `archived_at`) |
| `topic_unarchive` | Reverse archive (clears `archived_at`) |
| `topic_attach` | Bind a thread to a topic (recorded on the thread ref) |
| `topic_detach` | Remove a thread from a topic |
| `topic_alias` | Add or remove a topic alias (e.g. on rename) |
| `facet_set` | Change a thread's facet values (audited; see §7.3 for restrictions) |

There is intentionally no `topic_state` event in 2.0. If a richer topic lifecycle is added
later (F-W1), it will be introduced as a new additive event without breaking topics created
under 2.0.

### 2.5 Node

Unchanged from SPEC.md §4.3. Node types are preserved. Recording a decision is a node-level
action (typically a `summary` node) inside whatever thread reached the decision; there is no
thread-level `decision` facet (see §2.3.4).

### 2.6 Evidence

Unchanged from SPEC.md §4.4.

### 2.7 Actor

Unchanged from SPEC.md §2.6.

### 2.8 Approval

Unchanged from SPEC.md §2.7.

## 3. State machines

### 3.1 Topic lifecycle (no state machine in 2.0)

A topic has no formal state machine. Its lifecycle is a binary derived from `archived_at`:

| Derived state | Condition |
|---|---|
| `active` | `archived_at` is unset |
| `archived` | `archived_at` is set |

Transitions: `archive` and `unarchive` (idempotent).

#### 3.1.1 Derived topic summary

For UI and listing purposes, each topic exposes a derived **summary** computed from its child
threads. The summary is informational and does not appear in events.

| Summary | Condition |
|---|---|
| `empty` | Topic has no child threads |
| `has-open` | One or more child threads in non-terminal state |
| `all-terminal` | All child threads in `done`, `rejected`, or `deprecated` |

The richer red/yellow/green health model is deferred to F-W3 (Appendix A.3).

### 3.2 Thread state machine (unified)

A single state set replaces the four 1.x machines:

```text
draft -> open
draft -> withdrawn
open  -> working
open  -> rejected
open  -> withdrawn
working -> review
working -> rejected
review  -> done
review  -> working
review  -> rejected
done    -> open           # reopen
rejected -> open
done    -> deprecated
rejected -> deprecated
```

Initial state: depends on `lifecycle` (see §3.2.1).

#### 3.2.1 Lifecycle-filtered allowed states

The unified machine is filtered by the thread's `lifecycle` facet. Only the listed states are
reachable for each lifecycle:

| `lifecycle` | Allowed states | Initial | Notes |
|---|---|---|---|
| `proposal` | `draft`, `open`, `review`, `done`, `rejected`, `withdrawn`, `deprecated` | `draft` | `done` is the equivalent of 1.x `accepted` for RFCs |
| `execution` | `open`, `working`, `review`, `done`, `rejected` | `open` | `done` is the equivalent of 1.x `closed` |
| `record` | `open`, `done`, `rejected`, `deprecated` | `open` | Records short-lived; `working`/`review` skipped |

A transition not allowed for the thread's lifecycle is rejected with a clear hint.

#### 3.2.2 Mapping from 1.x states

Migration §10 specifies the 1.x → 2.0 state mapping. The mapping is lossless: every 1.x state has a
unique 2.0 equivalent.

### 3.3 State derivation

Unchanged from SPEC.md §3.5.

## 4. Data model

### 4.1 Topic

See §2.1 for fields.

### 4.2 Thread

See §2.2 for fields. Field `kind` from 1.x is **removed**; replaced by `facets`.

### 4.3 Event

See §2.4. New event types listed; existing event types unchanged.

### 4.4 Node types, Evidence, Approval

Unchanged from SPEC.md §4.3 / §4.4 / §4.5.

## 5. Storage layout

### 5.1 Git refs

Authoritative data in 2.0:

```text
refs/forum/topics/<TOPIC_ID>     # topic event chain (NEW)
refs/forum/threads/<THREAD_ID>         # thread event chain (unchanged structure)
refs/forum/aliases/<HANDLE>            # symref or note pointing to topic ID (NEW)
```

Topic handle resolution walks `refs/forum/aliases/<handle>` first; if absent, the handle is
treated as a topic ID and looked up under `refs/forum/topics/`.

### 5.2 Repository files

Same as SPEC.md §5.2 with added template:

```text
.forum/
  policy.toml
  actors.toml
  templates/
    topic.md         # topic charter template (NEW)
    thread.md           # generic thread template (NEW)
    proposal.md         # preset for lifecycle=proposal (replaces rfc.md)
    execution.md        # preset for lifecycle=execution (replaces task.md / issue.md)
    record.md           # preset for lifecycle=record   (replaces dec.md)
```

Old per-kind templates (`rfc.md`, `issue.md`, etc.) are deprecated but readable for migration.

### 5.3 Local files

Unchanged from SPEC.md §5.3.

## 6. Identity scheme

### 6.0 Type-marker symbols

User-facing identifiers carry a single-character **type marker** as the first character:

| Marker | Type | Storage form | Display form |
|---|---|---|---|
| `!` | topic handle | `<slug>` (alphanumeric + `-`) under `refs/forum/aliases/<slug>` | `!<slug>` |
| `@` | thread ID | `<8-char-base36>` under `refs/forum/threads/<token>` | `@<token>` |
| `/` | (separator) | n/a — display-only | `!<slug>/<index>` short reference |

Rationale:

- **`!` for topic** carries "named focus area / collection", visually distinct from `#` (which
  is conventionally a tag prefix and would conflict with the first-class `tags[]` concept in
  this spec).
- **`@` for thread** is **shell-safe** (no quoting needed), echoes the "at this address /
  conversation point" meaning, and visually contrasts with `!` so the two never blur in prose.
- The symbols appear only at the user-facing layer — refs, file paths, and serialized event
  fields use the bare token. This keeps Git ref-name validation rules (which forbid `!` mid-ref
  and reserve `@{` syntax) out of scope.
- `!` requires shell quoting (`'!payment-rewrite'`) because of bash history expansion. To keep
  interactive use friction-free, the bang is **optional at every CLI input position** — see
  §6.0.1 below. The `!` is **mandatory only in persisted / prose contexts** where type
  disambiguation matters (commit messages, body text, evidence refs, link targets).

This scheme gives every reference in commit messages, log output, and prose an unambiguous
visual type — an easy win over alphabetic prefixes (`wf-`, `t-`) that blur into the
identifier itself.

#### 6.0.1 Type-marker omission at CLI input

To avoid forcing users to quote `!` on every interactive command, the CLI **MUST accept** topic
and thread references with the leading marker omitted, whenever the surrounding command makes
the expected type unambiguous.

| Position | Bang/at required? | Notes |
|---|---|---|
| Positional or flag value of `topic show / topic ls / topic attach / topic detach / topic rename / topic archive / topic unarchive` | optional | The command grammar already requires a topic at this slot. `topic show payment-rewrite` is equivalent to `topic show '!payment-rewrite'`. |
| `--topic <ref>` flag on `thread new`, `thread ls`, etc. | optional | Type is fixed by the flag name. |
| Positional or flag value where a thread is required (`thread show`, `thread state`, `claim`, `evidence add`, etc.) | optional | `@` may be omitted for the same reason. `thread show a3f9b2k1` is equivalent to `thread show @a3f9b2k1`. |
| Mixed positions where either a topic or a thread is acceptable (e.g. `git forum show <REF>`) | **required** | Without the marker, the parser cannot disambiguate. Missing-marker input here returns `AmbiguousReferenceWithoutMarker` (§13) listing both candidate types. |
| Anywhere written into stored data (commit messages, evidence refs, link targets, body text) | **required** | The persisted-context rule (§9.6) is unchanged: bare tokens written into stored data are rejected as ambiguous. |

Error messages that surface a quoting failure SHOULD include a tip such as:

> Tip: drop the leading `!` to skip shell quoting — `topic show payment-rewrite` works the same.

This rule applies symmetrically to `@` for threads, even though `@` itself does not require
quoting. The benefit there is consistency, not friction reduction.

### 6.1 Topic handles

#### 6.1.1 Generation

When `git forum topic new "Payment rewrite"` is invoked:

1. Compute base slug: lowercase, strip non-alphanumerics, collapse hyphens, max 32 chars.
   `"Payment rewrite"` → `payment-rewrite`.
2. Candidate handle: `!payment-rewrite`.
3. **Within-clone collision** (the handle already exists locally): append a deterministic
   two-word petname derived from `sha256(topic_id)` (e.g. `!payment-rewrite-quick-fox`)
   and notify the user of the chosen handle in the command output. The petname dictionary is
   bundled (~2,048 adjectives × ~2,048 nouns ≈ 4M combinations; collision negligible).
4. **Cross-clone collision** (handle is unused locally but already claimed on the remote): not
   detected at this point — surfaces at push time as `HandleConflictOnPush` (§8.2.1) and
   requires explicit user rename. There is no silent auto-rename across clones.

User MAY override with `--handle !pay`. The override is validated against the handle format
and locally checked for collision. Within-clone petname appending also applies to overridden
handles. Cross-clone conflict still surfaces as `HandleConflictOnPush` regardless of whether
the handle was title-derived or user-overridden.

This deliberately splits the two failure modes:
- Within-clone collisions are mostly typos / accidental reuse and benefit from automatic
  petname recovery (low surprise; the user sees the result immediately).
- Cross-clone collisions involve another actor's claim and cannot be silently overridden
  without breaking the handle-as-stable-name guarantee.

#### 6.1.2 Reserved prefixes

Handles starting with `!_` (underscore as the first slug character) are reserved for future
system use. No
auto-allocated topic exists in 2.0; migration explicitly leaves 1.x threads as standalone (see
§10.1).

#### 6.1.3 Handle rename

`git forum topic rename <old> <new>`:

- Validates new handle availability and format.
- Records a `topic_alias` event keeping `<old>` as a permanent alias.
- Updates the SQLite index.
- `<old>` continues to resolve forever.

### 6.2 Thread IDs

**Display form**: `@XXXXXXXX` where `XXXXXXXX` is 8 base36 chars. The `@` is a type marker;
storage uses the bare `XXXXXXXX` under `refs/forum/threads/`. Generation algorithm and collision
analysis identical to SPEC.md §6.1, but the kind-prefix machinery is replaced by the type
symbol.

Legacy 1.x thread IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, etc.) remain valid for reading. The parser
accepts:

- `@XXXXXXXX` (2.0 native, display form)
- Bare `XXXXXXXX` (2.0 storage form, also accepted at CLI)
- Legacy `<KIND>-XXXXXXXX` (1.x opaque)
- Legacy `<KIND>-NNNN` (1.x sequential)

Unambiguous prefixes (≥4 chars after `@`) accepted as in 1.x.

### 6.3 Topic-scoped short references

Within a known topic context, `<handle>/<index>` references the Nth thread attached to the
topic (1-indexed by `topic_attach` event order). Examples:

```
git forum show !payment-rewrite/3
git forum show !payment-rewrite/design   # if a role label is assigned
```

Short references resolve to canonical thread IDs at parse time. They MUST NOT be stored as
canonical references in events or evidence (only canonical thread IDs are stored).

### 6.4 Canonical event/node IDs

Unchanged from SPEC.md §6.2 (Git commit OID).

## 7. Policy

### 7.1 Facet-scoped guards

Guard rules in 2.0 are scoped by **facet expression** instead of kind:

```toml
# 2.0: facet-scoped
[[guards]]
on = "lifecycle=proposal AND tag=cross-cutting : review->done"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

# 1.x equivalent (compat alias, internally rewritten):
[[guards]]
on = "rfc:under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]
```

The facet expression is a boolean over `lifecycle` and `tags` (using `tag=<value>` for membership
test). Unscoped guards (no facet expression) apply to all threads with the matching transition.

### 7.2 Operation checks

Operation check rule keys move from kind-named (`creation_rules.rfc`) to lifecycle-named, with
optional tag-specialization via dotted keys:

```toml
[creation_rules.proposal]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[creation_rules.execution]
required_body = false

[creation_rules.execution.tag.task]
required_body = true
body_sections = ["Background", "Acceptance criteria"]

[creation_rules.record]
required_body = true
body_sections = ["Context", "Decision", "Rationale", "Impact"]
```

Resolution: most-specific match wins. `creation_rules.execution.tag.task` overrides
`creation_rules.execution` for threads tagged `task`.

There are intentionally no topic-level guards in 2.0. See F-W2 (Appendix A.3) for the
forward-compatibility plan if these become needed.

### 7.3 Facet mutation rules

Changing a thread's facet values after creation MAY invalidate prior policy decisions. Rules:

- `lifecycle`: **immutable** after creation. Changing lifecycle requires creating a new thread via
  `--from-thread`.
- `tags`: mutable at any state. Tag changes that promote a thread into a stricter policy bucket
  (e.g., adding `task` triggers stricter `creation_rules.execution.tag.task`) re-evaluate
  operation checks and emit warnings if the thread no longer satisfies them.

## 8. Concurrency

git-forum 2.0 distinguishes two concurrency regimes:

- **Within-clone concurrency** — multiple processes on the same clone. Handled by Git's atomic
  ref CAS (compare-and-swap), as in 1.x.
- **Cross-clone concurrency** — independent writes on separate clones, reconciled at fetch/push
  time. The thread layer inherits 1.x's content-addressed IDs and semantic merge. The topic
  layer adds new conflict surfaces (handles, attach events, short indices, tags) that this section
  defines.

### 8.1 Within-clone protocol

Inherits SPEC.md §8 verbatim:

- `write_event` reads the current ref tip, creates a new commit, atomically updates the ref only
  if the tip has not changed.
- `create_ref` fails if the ref already exists.
- Concurrent writes to different threads/topics are fully safe.
- Concurrent writes to the same thread fail with a conflict error; the caller retries.

#### 8.1.1 Semantic merge (extended for 2.0)

Auto-merge cases, in addition to those in SPEC.md §8.1:

- Concurrent `topic_attach` to **the same topic** on the same thread (idempotent — second is
  a no-op).
- Concurrent `facet_set` events that change disjoint tag sets (additive merge).

Conflict cases that require the cross-clone resolution rules in §8.2:

- `topic_attach` to **different topics** (§8.2.2).
- `facet_set` events that add and remove the same tag (§8.2.4).

### 8.2 Cross-clone conflict resolution

Within-clone CAS does not protect against scenarios where two clones independently write
non-overlapping refs locally and only collide at push or fetch time. The following rules define
deterministic resolution.

#### Clock dependency

Several rules below order events by **wall-clock timestamp** (`event.timestamp`), with
`(actor_id, event_oid)` as deterministic tiebreakers. This means:

- Determinism across clones is **always guaranteed** — given the same set of events, every clone
  computes the same effective state.
- Correspondence between LWW order and **real time** assumes actor clocks are reasonably
  synchronized (NTP-grade skew of seconds, not minutes). A clone with a fast clock can win an
  LWW race against a clone whose write happened later in real time.
- Skew effects are bounded: the loser's intent is always preserved in event history (§8.1.1)
  and remains reversible by issuing a fresh write (re-attach, re-tag, etc.).

Adopting Hybrid Logical Clocks (HLC) to remove the wall-clock dependency is tracked as F-W5
(Appendix A.3).

#### 8.2.1 Topic handle conflict (push-time)

**Scenario.** Clone A creates topic `wA` with handle `!payment-rewrite`. Clone B
independently creates topic `wB` with the same handle. Both push.

**Why this happens.** Topic opaque IDs are content-addressed and never collide, but handles
are user-derived slugs (§6.1). A handle that appears unused on each clone may be claimed
elsewhere.

**Resolution.**

1. The topic event chain push (`refs/forum/topics/<TOPIC_ID>`) succeeds on both clones —
   different opaque IDs, no collision.
2. The alias ref push (`refs/forum/aliases/!payment-rewrite`) succeeds for whichever clone
   pushes first; the second clone's alias push fails (CAS against zero-SHA).
3. The losing client's atomic-push group (§8.4.1) fails as a whole. The push is reported as
   `HandleConflictOnPush` (an **error**, not a warning) with a message naming the existing
   claimant. **No automatic rename occurs.**
4. The user resolves the conflict explicitly by either:
   - `git forum topic rename <local-topic-id> <new-handle>` — pick a different handle,
     then re-push.
   - Decide the topic shouldn't have been created and `git forum topic archive` it
     locally before discarding (refs cleanup left to the user).

This deliberately blocks silent handle drift. A handle that a user has written into external
notes, RFC bodies, or commit messages must continue to mean what they wrote it to mean — silent
auto-rename of the loser would let a handle string be reassigned to a different topic without
the original author's knowledge, undermining the entire point of having a stable human-facing
handle.

**Interaction with `--handle <H>` user override.** A user-specified handle is treated as the
declared name. On cross-clone collision, the push fails with `HandleConflictOnPush` regardless
of whether the handle was title-derived or user-overridden — both are equally affected and
equally require explicit rename to resolve.

**CI / non-interactive contexts.** `HandleConflictOnPush` causes a non-zero exit; pipelines
treat it like any other push failure. A retry after `topic rename` is the explicit
remediation. There is no `--auto-rename` opt-in in 2.0; if dogfood evidence shows demand, it can
be added later behind an explicit flag (deferred).

#### 8.2.2 Topic attach conflict (fetch-time)

**Scenario.** Clone A attaches thread `@x9k2` to topic `!foo`. Clone B independently
attaches the same thread to `!bar`. Both push.

**Why this happens.** Both `topic_attach` events live on the thread ref chain. CAS protects
within a clone but not across clones writing in parallel.

**Resolution.**

1. First-push winner's attach event lands on the thread ref tip.
2. Second-push loser's attach event arrives via fetch; semantic merge appends it to the chain
   (event history preserves both intents).
3. **Effective topic membership** is determined by replaying **all** `topic_attach` and
   `topic_detach` events on the thread and selecting the most recent by:
   - Primary key: `event.timestamp` (actor clock at write time).
   - Tiebreaker: lexicographic order of actor ID.
   - Final tiebreaker: lexicographic order of event OID.

   - If the most recent event is `topic_attach W`, the thread's effective topic is `W`.
   - If the most recent event is `topic_detach`, the thread is standalone (no topic).
4. Both `topic show` (on the losing topic) and `thread show` surface
   `AttachConflictResolved` as an informational warning so the discrepancy is visible.

`topic_detach` participates in the same LWW ordering as `topic_attach`: a detach event
with a later timestamp than a competing attach event wins (resulting in standalone), and vice
versa. This unifies the rule across attach/detach without separate semantics.

A user who disagrees with the auto-resolution issues `topic detach` on the losing side and
re-attaches as desired; this records new events that supersede the auto-resolution.

#### 8.2.3 Handle alias divergence

Two sub-scenarios.

**Scenario A: rename ⊕ create.** Clone A renames `!foo` → `!bar` (alias ref `!bar` now
points to A's topic). Clone C independently creates a new topic with handle `!bar`.

**Resolution.** Identical to §8.2.1: the second pusher's alias ref fails CAS and the push is
reported as `HandleConflictOnPush`. The user (whichever pushes second) must explicitly rename
their topic before re-pushing. No silent auto-resolution occurs.

**Scenario B: divergent rename of the same topic.** Clone A renames `!foo` → `!bar`.
Clone B independently renames the same topic `!foo` → `!baz`.

**Resolution.**

1. Both `topic_alias` events are recorded on the topic ref. The thread-style CAS protocol
   serializes them within each clone; cross-clone, both events land on the topic event chain
   via semantic merge.
2. Both new alias refs (`!bar` and `!baz`) are created locally on the issuing clone and
   pushed. Both succeed (different ref names; neither pre-exists). The original alias ref
   `!foo` is preserved by both (per §6.1.3 — old handles never expire).
3. After sync, **all three handles** (`!foo`, `!bar`, `!baz`) resolve to the same
   topic.
4. The **primary** handle (the one shown by default in `topic show` and `topic ls`) is
   the most recent `topic_alias` event by LWW order: `(timestamp, actor_id, event_oid)`.
   The other handles are surfaced as alternates.

No conflict from the user's perspective — both rename intents succeed. The display preference
follows LWW.

#### 8.2.4 Tag merge semantics

**Scenario.** Clone A adds tag `urgent` to thread `@x9k2`. Clone B concurrently removes tag
`urgent` (or adds a different tag).

**Resolution.**

1. All `facet_set` events are preserved in the thread's event chain.
2. The derived `tags` set is computed by replaying `facet_set` events in
   `(timestamp, actor_id, event_oid)` order, applying add/remove per event.
3. Last-write-wins per individual tag — the most recent event mentioning a given tag determines
   whether it is present.

This matches the LWW semantics used for attach conflicts (§8.2.2) and avoids requiring user
intervention for ordinary tag drift.

> **Note on tag CRDTs.** A pure observed-remove CRDT would eliminate the wall-clock dependency
> entirely for tag merging. LWW is chosen for implementation simplicity and consistency with the
> attach rule; tag drift in the LWW model is theoretically possible but bounded (the next
> explicit `tag add`/`tag rm` always wins). CRDT-based tag semantics are tracked as F-W6
> (Appendix A.3).

#### 8.2.5 Archived topic with concurrent attach

**Scenario.** Clone A archives topic `!foo` (writes `topic_archive` event on the topic
ref). Clone B, having not yet seen the archive, writes `topic_attach` on a thread referencing
`!foo`.

**Resolution.**

- Within a clone, an attach to an archived topic is **rejected** with
  `AttachToArchivedTopic` (§13). `--force` overrides explicitly. Because archived topics
  are hidden by default in `ls` (§9.3), this prevents work from silently disappearing into a
  topic nobody is looking at.
- Across clones, both events succeed at the ref layer (different refs). After fetch, if the
  attach event was written before the archive was visible locally, the resulting state is:
  - Topic `!foo` is `archived`.
  - Thread lists `!foo` as its topic (the attach was not blocked locally because archive
    was unseen).
  - `doctor` reports the inconsistency and recommends explicit detach or unarchive.

In 2.0 the user resolves cross-clone inconsistencies manually. Future topic-level guards
(F-W2, Appendix A.3) MAY introduce stricter automated remediation.

### 8.3 Short-index stability across clones

Topic short indices (`!foo/3`, §2.1.3) are **derived, session-local references**, not
canonical IDs:

- The mapping is computed at query time from locally-visible `topic_attach` events sorted by
  `(attach_event.timestamp, actor_id, event_oid)`.
- Before two clones have fully synced, they may compute different `/N` values for the same
  thread.
- After sync, the ordering is deterministic across clones.
- `/N` MUST NOT appear in stored data: not in evidence refs, not in link targets, not in commit
  messages used by hooks. Only canonical thread IDs (`@XXXXXXXX`) and topic handles are
  stored.

Tooling MAY warn when a `/N` reference is used in a context that would be persisted (e.g., a
commit message intended for the `commit-msg` hook).

### 8.4 Push/fetch protocol

#### 8.4.1 Atomic ref groups

Some logical operations span multiple refs and MUST be pushed atomically to avoid leaving the
remote in a state that other clients can observe as inconsistent. Clients use Git's
`push --atomic` option (or equivalent transport semantics) to enforce this.

| Logical operation | Refs in the atomic group |
|---|---|
| `topic_create` | `refs/forum/topics/<TOPIC_ID>` + `refs/forum/aliases/<HANDLE>` |
| `topic_rename` | `refs/forum/topics/<TOPIC_ID>` (recording the `topic_alias` event) + `refs/forum/aliases/<NEW_HANDLE>` |
| `topic_archive` / `topic_unarchive` | `refs/forum/topics/<TOPIC_ID>` only |
| Thread events (create, say, attach, detach, state, facet_set, etc.) | `refs/forum/threads/<THREAD_ID>` only |

Atomic push is **mandatory** for the topic-create and topic-rename groups: a non-atomic
push that succeeds only on the topic ref but fails on the alias ref would leave a topic
without a handle visible to other clients (or, worse, leave a dangling alias if the order
were reversed). Clients that cannot guarantee atomic push MUST refuse the operation rather than
proceed.

#### 8.4.2 Recommended push order within a session

When pushing many independent operations, ordering does not affect correctness (each atomic
group is self-contained), but for fastest conflict surfacing the recommended order is:

1. Topic create / rename groups (so handle conflicts surface early).
2. Thread events (so attach references are valid against just-pushed topics).
3. Pure topic events (archive, etc.).

#### 8.4.3 Fetch

Fetch always pulls all three ref trees (topics, threads, aliases). After fetch, the local
SQLite index is rebuilt incrementally to reflect any attach / tag / handle changes. Conflicts
surfaced by §8.2 are reported by `git forum doctor` and on the next interactive `show` of
affected topics / threads.

## 9. CLI surface

### 9.1 Topic commands (NEW)

```text
git forum topic new <TITLE> [--handle <HANDLE>]
    [--body <TEXT> | --body-file <PATH> | --edit]
git forum topic show <TOPIC>
git forum topic ls [--archived] [--summary <SUMMARY>]
git forum topic attach <TOPIC> <THREAD>...
git forum topic detach <TOPIC> <THREAD>...
git forum topic rename <OLD_HANDLE> <NEW_HANDLE>
git forum topic archive <TOPIC>
git forum topic unarchive <TOPIC>
```

- `--archived` includes archived topics (default: hidden in `ls`).
- `--summary <empty|has-open|all-terminal>` filters by derived summary.
- There is no `git forum topic state` command — topic has no state machine.
- `topic attach` to an archived topic is **rejected** with `AttachToArchivedTopic`
  unless `--force` is passed. Rationale: archived topics are hidden by default in `ls`, so a
  silent attach would put work in a place users won't see.

#### 9.1.1 `topic show` output

`topic show` displays a header block followed by the attached threads grouped by lifecycle. The
header MUST surface publication state so the user knows whether a handle is shared with the
remote:

| Header line | When shown |
|---|---|
| `Topic: !<handle>` | always |
| `Title: <title>` | always |
| `State: active` / `State: archived (since <date>)` | always |
| `Charter: <first non-empty body line>` / `Charter: (none)` | always |
| `Summary: <empty / has-open / all-terminal> (<n> threads attached)` | always |
| `(LOCAL ONLY — not pushed to any remote)` | the topic's `topic_create` event has never been observed on any tracked remote ref. Removed once a successful push lands. |
| `(LOCAL ONLY — handle not pushed: <reason>)` | the topic event chain is on the remote, but the alias ref failed to push (`HandleConflictOnPush`, §8.2.1). Reason text names the claimant of the conflicting handle. |
| `Pending divergence: <N> attach conflict(s) — see 'doctor'` | `AttachConflictResolved` warnings (§8.2.2) exist and have not been acknowledged. |

The publication-state lines fire only when the relevant condition is true; in the steady-state
shared-and-clean case the header is the four standard rows plus the summary.

This makes the Day-4-style "I pushed and got an error, now what?" recovery path discoverable
from `topic show` alone, without requiring the user to remember the original push log.

### 9.2 Thread commands (unified + presets)

Canonical form:

```text
git forum thread new <TITLE>
    --lifecycle <LIFECYCLE>
    [--topic <TOPIC>] [--tag <TAG>...]
    [--body <TEXT> | --body-file <PATH> | --edit]
    [--branch <BRANCH>] [--link-to <THREAD> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD>] [--force]
git forum thread show <THREAD>
git forum thread ls [--topic <TOPIC>]
    [--lifecycle <LIFECYCLE>]
    [--status <STATUS>] [--tag <TAG>] [--branch <BRANCH>]
git forum thread state <THREAD> <NEW_STATE> [--approve <ACTOR>]... [--comment <TEXT>]
git forum thread tag add <THREAD> <TAG>...
git forum thread tag rm  <THREAD> <TAG>...
```

Kind presets — **stable, first-class commands** (not compat aliases). They are the everyday
surface; the canonical `thread new --lifecycle ...` form above is reserved for power-users and
scripts that want to set arbitrary facet/tag combinations.

```text
git forum new rfc   <TITLE>    → --lifecycle proposal  --tag cross-cutting
git forum new dec   <TITLE>    → --lifecycle record
git forum new task  <TITLE>    → --lifecycle execution --tag task
git forum new issue <TITLE>    → --lifecycle execution --tag bug
git forum new bug   <TITLE>    → --lifecycle execution --tag bug    (alias of `new issue`)
```

`--help` for both forms shows the other as a cross-reference. Presets remain supported across
all 2.x and 3.x releases — they are not on the removal schedule. Only kind-prefixed thread IDs
(`RFC-0001`) and kind-keyed policy keys (`creation_rules.rfc`) are deprecated by ADR-004.

### 9.3 Listing and display

```text
git forum ls                            # default: mixed view (active topics + standalone inbox)
git forum ls --topics                # active topics only
git forum ls --threads                  # all threads, flat
git forum ls --inbox                    # standalone threads only (no topic attached)
git forum show <REF>                    # auto-detects topic vs thread
```

The default `ls` is a **mixed view** with two stacked sections:

1. **Topics** — non-archived topics with their derived summary and thread count.
2. **Inbox** — standalone (unattached) threads, sorted by `updated_at` desc.

This default ensures that newly captured threads (the most common case immediately after
migration, and the common case for quick bug capture in steady state) are visible without
needing to remember a flag. Users who prefer a single view can use `--topics`,
`--threads`, or `--inbox`.

### 9.4 Discussion, lifecycle, evidence, links, hooks

Unchanged from SPEC.md §9.4 / §9.5 / §9.7 / §9.10. State-change shorthand commands (`close`,
`accept`, etc.) continue to work and map to the unified state machine via the thread's lifecycle
facet:

| Shorthand | `lifecycle=execution` | `lifecycle=proposal` | `lifecycle=record` |
|---|---|---|---|
| `close` | → `done` | (rejected: use `accept`) | → `done` |
| `accept` | (rejected: use `close`) | → `done` | → `done` |
| `propose` | (rejected) | → `open` (from `draft`) | (rejected) |
| `pend` | → `working` | (rejected) | (rejected) |
| `reject` | → `rejected` | → `rejected` | → `rejected` |
| `deprecate` | → `deprecated` | → `deprecated` | → `deprecated` |

Shorthand commands work uniformly on **both topic-attached and standalone threads**. Topic
membership is never required for state changes. (Future topic-level guards — F-W2 in
Appendix A.3 — would only add constraints for attached threads; standalone threads remain
unconstrained by topic policy.)

### 9.5 Preflight, doctor

`git forum verify <THREAD>` and `git forum doctor` continue to work; both gain topic-aware
output:

- `verify` for a thread inside a topic notes the topic handle and summary.
- `doctor` reports:
  - **Untriaged standalone threads** (no topic attached) as an informational count, not a
    warning. Standalone is a legitimate steady state — many bugs and notes never need a
    topic. The doctor output names this section "Untriaged" rather than "Orphan" to reflect
    that this is normal state, not a fault.
  - Broken aliases, dangling attach references, and unresolved cross-clone conflicts
    (`AttachConflictResolved` per §8.2.2) — these *are* warnings.
- (No topic-level guard preview in 2.0; see F-W2.)

### 9.6 Persisted-context validation

Topic short references (`/N`, §2.1.3) MUST be rejected as input wherever the value would be
stored. The check fires `ShortIndexInPersistedRef` (an error per §13). Implementations enforce
the check at every entry point listed below.

| Entry point | Why persisted | Rejection point |
|---|---|---|
| `git forum link --to <ref>` | stored as link target on the thread ref | argument parser |
| `git forum link <from> <to>` | both endpoints stored | argument parser |
| `git forum evidence add --kind thread --ref <ref>` | stored as evidence reference | argument parser |
| `git forum topic attach <topic> <thread>` | the thread positional must be a canonical thread ID; `/N` here would be circular | argument parser |
| `git forum topic detach <topic> <thread>` | same as attach | argument parser |
| `--link-to <ref>` (any creation command) | stored as link target | argument parser |
| `--from-thread <ref>` (any creation command) | recorded in the new thread's source link | argument parser |
| `commit-msg` hook input | scanned and validated against the forum index | hook handler |
| `--topic <ref>` for `thread new` | accepts only canonical topic handles, not `!foo/N` (which is a thread reference, not a topic reference) | argument parser |

`/N` is **accepted** at:

- `git forum show <ref>` — read-only display.
- `git forum thread state <ref> <new-state>` — interactive state change; the state event is
  recorded against the canonical thread ID resolved from `/N` at parse time.
- All other read-only / interactive query commands where the resolved canonical ID is used
  internally and not echoed into stored data.

Free-form body text (`--body`, `--body-file`, `--edit` content) is **not scanned** for `/N`.
Authors who write `!foo/3` in prose are responsible for prose accuracy; the rule covers
machine-interpreted references only.

#### Error UX requirement: canonical-ID suggestions

When `ShortIndexInPersistedRef` fires, the rejection MUST include the **canonical thread ID**
that the short reference would have resolved to, so the user can fix the input by copy-paste
without re-running `topic show`. Example commit-msg hook output:

```
git-forum: error: commit message references a short index that cannot be persisted

  found:    !node-id-scheme-review/3
  resolved: @d8f4q9aa  (slot /3 of '!node-id-scheme-review' on this clone)

  Replace the short reference with the canonical thread ID and re-commit.
  (Short references like '/3' are display-only; cross-clone they may point
  to different threads.)
```

If the reference cannot be resolved at the moment of rejection (e.g., the topic itself does not
exist locally), the error message MUST say so explicitly and recommend `topic show` rather than
silently failing with "unknown reference".

## 10. Migration from 1.x

### 10.1 Strategy

Hard break with one-shot migration plus a short-term compatibility alias layer.

```text
git forum migrate         # rewrites refs in place; produces migration log
git forum migrate --dry-run
```

After migration:

- Existing thread refs are rewritten: `refs/forum/threads/RFC-0001` → `refs/forum/threads/t-...`
  with an alias entry mapping the old name (so external links keep resolving).
- Each thread gets a `facet_set` event added to its history populating `lifecycle` and the
  conventional `tags` (`cross-cutting` for `rfc`; `bug` for `issue`; `task` for `task`) per the
  §2.3.3 mapping.
- States are remapped per §3.2.2.
- **Migrated threads remain standalone** (no topic attachment). No `!_legacy` topic is
  auto-created. Users attach threads to topics manually as they triage. `doctor` reports the
  orphan count after migration as an informational signal — it will be high initially and is
  expected to decrease over time as triage proceeds.

Rationale for leaving threads orphan rather than auto-bucketing: a synthetic `!_legacy`
topic would pollute the `topic ls` output indefinitely (since users rarely empty it
completely) and creates a misleading impression that legacy threads form a coherent workstream.
Standalone is the honest representation of "uncurated work".

### 10.2 What is permanent vs deprecated

**Permanent (no removal scheduled):**

- Top-level kind-named commands: `git forum new rfc/dec/task/issue/bug` and the corresponding
  `close` / `accept` / `pend` / `propose` / `reject` / `deprecate` shorthands. These are the
  stable everyday surface (§9.2).

**Deprecated (removal scheduled per §10.4):**

- Kind-prefixed *subcommand* forms — `git forum rfc new`, `git forum issue close`, etc. — work
  as silent aliases in 2.0. These were the 1.x grouping convention and are superseded by the
  top-level forms above.
- Kind-prefixed thread IDs (`RFC-0001`, `ASK-XXXXXXXX`) resolve via the alias table for read.
- Kind-keyed policy keys (`creation_rules.rfc`, `[[guards]] on = "rfc:..."`) auto-rewrite to
  lifecycle keys at load time with a warning.

### 10.3 What does NOT migrate automatically

- Custom guard rules in `policy.toml` using kind-scoped `on = "rfc:..."` keys are auto-rewritten,
  but custom rules that mention kinds in user-defined functions require manual update.
- TUI custom keybindings referencing `kind` (none exist in shipped configs, but document the risk).

### 10.4 Removal schedule

Applies to **deprecated** items only (§10.2). The kind-named top-level commands
(`new rfc/task/bug/dec`, `accept`, `close`, etc.) are permanent and **not** subject to this
schedule.

| Version | Kind-prefixed subcommands | Kind-keyed policy | Legacy IDs |
|---|---|---|---|
| 2.0 | silent alias, `--help` cross-references | auto-rewrite + warning | resolve via alias |
| 2.1 | warn on use | unchanged | resolve via alias |
| 3.0 | removed | rejected (must be migrated) | read-only resolve |

## 11. TUI

The TUI default home view mirrors the CLI's mixed-listing default (§9.3):

1. **Mixed home** (default) — two stacked panels:
   - Top: active (non-archived) topics with handle, summary, and child count.
   - Bottom: standalone (unattached) threads, sorted by `updated_at` desc.
   Both panels are simultaneously navigable; `Tab` switches focus between them.
2. **Topic detail** showing the topic's charter (body) and attached threads grouped by
   lifecycle.
3. **Thread detail** (unchanged from 1.x in structure, but shows facets in header).
4. **Single-mode views** for users who prefer one section at a time — `W` keybinding
   (topics only), `T` keybinding (all threads, flat), `I` keybinding (inbox / standalone
   only).

The mixed default ensures fresh captures (the common case immediately after migration and the
common case for quick bug capture in steady state) are visible without keyboard rituals.

Topic archive/unarchive is a single-key action from the topic detail view.

## 12. Search

Search index gains:

- `topic_handle`, `topic_archived`, `topic_summary` columns on threads.
- A `lifecycle` column and a `tags` join table replacing the `kind` column.

Old search queries referencing `kind:rfc` are auto-translated to
`lifecycle:proposal AND tag:cross-cutting` for one minor release. `kind:issue` translates to
`lifecycle:execution AND tag:bug`; `kind:task` to `lifecycle:execution AND tag:task`; `kind:dec`
to `lifecycle:record`.

## 13. Error handling

Unchanged from SPEC.md §13. New error and warning categories:

| Code | Severity | Triggered by | Notes |
|---|---|---|---|
| `TopicNotFound` | error | handle resolution failure | Lists similar handles |
| `ThreadNotInTopic` | error | `<handle>/N` index out of bounds | |
| `FacetTransitionDisallowed` | error | facet mutation in a state that doesn't allow it | |
| `LifecycleStateMismatch` | error | state transition not allowed for thread's lifecycle | |
| `HandleConflictOnPush` | **error** | alias ref CAS failure on push (§8.2.1, §8.2.3) | Atomic push group fails; user must `topic rename` and re-push |
| `AttachToArchivedTopic` | **error** | attach attempt to a topic whose `archived_at` is set | `--force` overrides; intentional gate to keep work visible |
| `AttachConflictResolved` | warning | divergent `topic_attach`/`topic_detach` reconciled by LWW (§8.2.2) | Surfaced in `show` until manually re-attached or acknowledged |
| `ShortIndexInPersistedRef` | **error** | `/N` short reference appears where it would be stored (e.g. commit message scanned by `commit-msg` hook, evidence ref, link target) | Error message MUST include the canonical thread ID resolved at the moment the short reference was rejected (e.g., "did you mean `@d8f4q9aa`?"). Resolving requires reading the topic at write time, but the rejection itself is preflight. |
| `AmbiguousReferenceWithoutMarker` | error | Bare token (no `!`/`@`) used in a CLI position that accepts both a topic and a thread (e.g. `git forum show <REF>`) | Lists candidate topic / thread matches; suggests prefixing with `!` or `@` to disambiguate. |

## 14. Testing strategy

Unchanged from SPEC.md §14, plus:

### Migration

- Every state in every 1.x kind round-trips to a defined 2.0 state.
- Migrated threads remain standalone (no synthetic `!_legacy` topic created).
- Default `git forum ls` post-migration shows all migrated threads in the inbox section.

### Facet model

- Facet expression evaluator tests covering all guard scoping forms (`lifecycle=...`,
  `tag=...`, `AND`/`OR`/`NOT`).
- Kind preset commands (`new rfc/dec/task/bug`) produce identical facet/tag combinations as the
  canonical `thread new --lifecycle ... --tag ...` form.
- A topic can hold threads of all three lifecycles simultaneously; `topic show` groups
  them correctly.

### Cross-clone concurrency

- Each of §8.2.1–§8.2.5 reproduced with two simulated clones.
- §8.2.1 / §8.2.3 (handle conflict): the second push **fails with `HandleConflictOnPush`**
  (no auto-rename); after explicit `topic rename` on the loser, the second push succeeds.
- §8.2.2 (attach LWW): combined attach + detach event sequences resolve to the most recent
  event by `(timestamp, actor_id, event_oid)` order; `AttachConflictResolved` warning surfaced.
- §8.2.4 (tag LWW): per-tag LWW result is independent of event arrival order.
- §8.2.5 (archived attach): within-clone attach to archived topic rejected with
  `AttachToArchivedTopic`; `--force` overrides; cross-clone attach written before archive
  visibility is preserved with doctor warning.

### CLI / UX defaults

- Default `git forum ls` returns mixed topics + inbox; `--topics` / `--threads` /
  `--inbox` produce single-section variants.
- `doctor` reports standalone-thread count under "Untriaged" (informational), not warning.
- Standalone threads accept all state-change shorthands (`close`, `accept`, `pend`, ...) without
  topic attachment.

### Type-marker omission (§6.0.1)

- `git forum topic show payment-rewrite` resolves identically to
  `git forum topic show '!payment-rewrite'`.
- `git forum thread show a3f9b2k1` resolves identically to `git forum thread show @a3f9b2k1`.
- `git forum show <bare-token>` returns `AmbiguousReferenceWithoutMarker` when the token
  matches both an existing topic slug and a thread token (lists both candidates).
- Shell-quoting failure messages include the "drop the leading `!`" tip.

### `/N` short-index validation

- Two clones with diverging attach order produce the same `/N` mapping after sync.
- `/N` accepted as input to read-only CLI commands (`show`, etc.).
- `/N` **rejected with `ShortIndexInPersistedRef`** in every persisted-context check point
  enumerated in §9.6.
- The rejection message includes the canonical thread ID the short reference resolved to
  (e.g. `@d8f4q9aa`); when the topic itself does not resolve locally, the message says so
  explicitly instead of returning an opaque "unknown reference" error.

### `topic show` publication-state header

- A topic created locally but never pushed shows `(LOCAL ONLY — not pushed to any remote)` in
  the header.
- A topic whose alias ref failed to push (`HandleConflictOnPush`) shows
  `(LOCAL ONLY — handle not pushed: ...)` naming the claimant.
- After successful push of both event chain and alias, the LOCAL ONLY line disappears.
- Unacknowledged `AttachConflictResolved` warnings produce a `Pending divergence: <N>` line
  in the header.

## 15. Non-goals

In addition to SPEC.md §15:

- General-purpose project management (Gantt charts, dependency graphs across topics).
- Topic state machines, topic-level guards, topic nesting in 2.0
  (intentionally deferred — see Appendix A.3).
- Multi-parent topics (DAG of topics).
- User-defined required facet axes beyond `lifecycle` (use `tags` instead).
- Mandatory topic membership for threads.

## Appendix A: Open questions

### A.1 Resolved during 2.0 drafting

| ID | Question | Resolution |
|---|---|---|
| O-1 | Should `!_legacy` migration bucket be created automatically, or should threads stay orphan until manually attached? | **Orphan**. No synthetic topic on migration; `doctor` reports orphan count. (§10.1) |
| O-2 | Are 5 intent values enough? | **Dropped entirely**, and `scope` was dropped too. Sole required facet is `lifecycle`; everything else (bug/task/cross-cutting) is a tag. (§2.3) |
| O-3 | Should standalone threads be allowed to use shorthand commands (`close`, `accept`) directly? | **Yes**. Shorthand commands work uniformly on standalone and attached threads. Topic attachment is never required for state changes. (§9.4) |

### A.2 Remaining for 2.0 implementation

(none currently outstanding — to be added as implementation surfaces design questions)

### A.3 Deferred from Level XS scoping (forward-compatibility plan)

The following capabilities were considered for 2.0 and **deliberately deferred** to keep the
release scope tight. Each can be added in a 2.x minor release without breaking 2.0 clients,
provided the additive contracts below are honored.

| ID | Capability | Current 2.0 substitute | Trigger to add | Forward-compat contract |
|---|---|---|---|---|
| F-W1 | Topic state machine (e.g. `planning` / `active` / `wrapping` / `done` / `abandoned`) | `archived_at` flag + derived summary | Need to express stage of work as a queryable signal beyond "active vs archived" | Introduce `topic_state` event type (additive). Topics without any `topic_state` event default to `active`. `archived` remains derived from `archived_at` and is orthogonal to status. |
| F-W2 | Topic-level guards | None (rely on per-thread guards) | Need to enforce conditions on topic archival or future state transitions (e.g. "all children terminal before archive") | Add `[[topic_guards]]` policy section. Guards on `unrestricted` operations (archive in 2.0) are absent by default; adding rules later affects only repos that opt in. |
| F-W3 | Richer derived health (`green` / `yellow` / `red`) replacing the simple summary | `empty` / `has-open` / `all-terminal` | Need to surface "stuck" topics visually (e.g. unresolved objections, stale activity) | Health is a pure function of child state. Richer logic adds without breaking simpler clients; index columns gain a `topic_health` field, summary remains for backward queries. |
| F-W4 | Topic nesting (single-parent) | Flat topics only | Need to express epic / sub-topic hierarchy | Add optional `parent` field to topics. Absent = root. Cycles rejected at write time. Existing 2.0 topics are roots by default. |
| F-W5 | Hybrid Logical Clocks (HLC) for cross-clone event ordering | Wall-clock LWW with `(actor_id, event_oid)` tiebreak (§8.2 clock-dependency note) | Observed clock skew producing user-surprising LWW outcomes; multi-region deployments | Add HLC field to event metadata as additive serialization. Clients that don't compute HLC continue to fall back to wall-clock; clients that do prefer HLC. Both populations converge to the same effective state once events propagate. |
| F-W6 | CRDT-based tag merging (observed-remove set) | Per-tag LWW (§8.2.4) | Observed tag flicker across clones causing confusion in dashboards / agent decisions | Replace the `facet_set`-event replay logic with OR-set semantics. Event format unchanged; merge function swap is internal. Old clients computing LWW agree with new clients computing OR-set in the absence of concurrent add/remove on the same tag. |

#### Why Level XS over Level XXS

Level XXS would have collapsed `topic` to a tag-like string field on threads, eliminating the
topic ref tree entirely. That model is simpler still, but it would have made F-W1 and F-W4
**re-architecture** changes (introducing a new entity), not additive ones. Level XS preserves
topic as a first-class entity so all four future capabilities above remain forward-compatible.

#### Trigger discipline

A future minor release SHOULD add an F-Wn capability only when:

1. Documented dogfood evidence shows the substitute is insufficient.
2. The additive contract above is honored (no breaking change for clients on prior minor).
3. The corresponding ADR is written and accepted.

Speculative implementation of F-W1–F-W6 without these triggers is explicitly discouraged.

## Appendix B: Examples

These examples illustrate the 2.0 model end-to-end. Output formatting is illustrative; exact
column layouts may differ.

### B.1 Quick bug capture (standalone, no topic)

```text
$ git forum new bug "TUI crashes on terminal resize"
created thread @a3f9b2k1
  lifecycle:  execution
  tags:       bug
  status:     open
  topic:   (standalone)

$ git forum claim @a3f9 "Resize handler doesn't account for negative width on shrink"
appended say:claim node n-5h2m9p1k

$ git forum evidence add @a3f9 --kind file --ref src/tui/render.rs:42
appended evidence n-7c4d8e3a

# After fix lands:
$ git forum close @a3f9 --comment "Fixed in commit 7c8d2e1"
state: open -> done
```

The thread never joined a topic. Standalone use is fully supported (see §9.4).

### B.2 RFC inside a topic

```text
$ git forum topic new "Payment system rewrite"
created topic !payment-system-rewrite
  active, no charter

$ git forum new rfc "Replace synchronous gateway with async queue" \
    --topic !payment-system-rewrite --edit
created thread @x9k2m4p7
  lifecycle:  proposal
  tags:       cross-cutting
  status:     draft
  topic:   !payment-system-rewrite (slot /1)

$ git forum question @x9k2 "How do we handle ordering invariants in the queue?"
$ git forum objection @x9k2 "Async retries can violate at-most-once delivery"

# After review:
$ git forum resolve @x9k2 n-9b3c4d5e
$ git forum summary  @x9k2 "Decision: queue-based dispatch with idempotency keys"
$ git forum propose  @x9k2          # draft -> open
$ git forum state    @x9k2 review   # open -> review
$ git forum accept   @x9k2 --approve human/alice
state: review -> done
```

Note that `--approve human/alice` satisfies the
`lifecycle=proposal AND tag=cross-cutting : review->done` guard from §7.1.

### B.3 Implementation task linked to the RFC

```text
$ git forum new task "Implement async queue dispatcher" \
    --topic !payment-system-rewrite \
    --link-to @x9k2 --rel implements \
    --branch feat/async-dispatcher
created thread @y3p7n2q4
  lifecycle:  execution
  tags:       task
  status:     open
  topic:   !payment-system-rewrite (slot /2)
  branch:     feat/async-dispatcher

# Commits on feat/async-dispatcher reference @y3p7 in their messages.
# When merged:
$ git forum close @y3p7
state: open -> done
```

### B.4 Lightweight decision record (standalone)

```text
$ git forum new dec "Use UUIDv7 for new entity IDs" --edit
created thread @q8w2e1r3
  lifecycle:  record
  tags:       (none)
  status:     open
  topic:   (standalone)

$ git forum close @q8w2
state: open -> done
```

`lifecycle=record` skips the `working` / `review` states — records go straight to `done`.

### B.5 Listing — default mixed view

```text
$ git forum ls
TOPICS (active)
  HANDLE                          SUMMARY        THREADS
  !payment-system-rewrite       has-open       3
  !onboarding-revamp            all-terminal   12

INBOX (standalone, sorted by updated_at desc)
  ID         TITLE                                       LIFECYCLE  TAGS    UPDATED
  @a3f9b2   TUI crashes on terminal resize              execution  bug     2026-04-28
  @q8w2e1   Use UUIDv7 for new entity IDs               record     -       2026-04-27
  @7m4k9p   How does retry policy interact with quotas? execution  bug     2026-04-26

$ git forum ls --topics --archived           # explicit archived view
HANDLE                          SUMMARY        THREADS  ARCHIVED
!q1-perf                      empty          0        2026-02-15

$ git forum thread ls --lifecycle execution --tag bug --status open
ID         TITLE                                  TOPIC                     CREATED
@a3f9b2   TUI crashes on terminal resize         (standalone)              2026-04-25
@r7n8m1   Auth retry loop on token refresh       !payment-system-rewrite 2026-04-26

$ git forum show !payment-system-rewrite
Topic: !payment-system-rewrite
Title:    Payment system rewrite
State:    active
Summary:  has-open  (3 threads attached)

Threads:
  /1 [proposal/done   ] Replace synchronous gateway with async queue   @x9k2m4
  /2 [execution/done  ] Implement async queue dispatcher               @y3p7n2
  /3 [execution/open  ] Migrate gateway clients                        @d8f4q9
```

The default `git forum ls` shows both stacked sections, ensuring that newly captured standalone
threads remain visible without needing to remember a flag.

### B.6 Promoting a standalone thread

```text
$ git forum topic attach !payment-system-rewrite @a3f9b2
attached @a3f9b2 to !payment-system-rewrite (slot /4)
```

The thread is no longer standalone; it now appears in the topic's `show` output.

### B.7 Cross-clone handle conflict (explicit resolution)

```text
# Alice and Bob both created a topic with the same title "Payment rewrite"
# locally (different opaque IDs). Alice pushed first.

bob$ git push
error: HandleConflictOnPush: handle '!payment-rewrite' is already claimed
       by topic id x9k2m4p7 (created 2026-04-28T09:11:02Z by ai/alice).

       Your topic (internal id aaaa1234, currently published as !payment-rewrite
       on this clone only) was not pushed. Resolve by renaming:

         git forum topic rename !payment-rewrite <new-handle>
         git push

bob$ git forum topic rename !payment-rewrite !payment-rewrite-bob
renamed: handle is now !payment-rewrite-bob
(the conflicting local name '!payment-rewrite' was never published from this clone)

bob$ git push
ok
```

No silent reassignment occurs. Bob's choice of new handle is explicit; Alice's
`!payment-rewrite` continues to mean exactly what she expected it to mean.

### B.8 Tag-driven policy customization

```toml
# .forum/policy.toml
[creation_rules.execution]
required_body = false                  # bugs can be one-liners

[creation_rules.execution.tag.task]
required_body = true                   # but tasks need structured bodies
body_sections = ["Background", "Acceptance criteria"]

[creation_rules.proposal]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[[guards]]
on = "lifecycle=proposal AND tag=cross-cutting : review->done"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "lifecycle=proposal : review->done"
requires = ["at_least_one_summary"]    # all proposals need a summary; cross-cutting also need approval
```

When a thread tagged `task` is created without acceptance criteria, the operation check fires a
warning (or error if `strict = true`).

## Appendix C: References

- SPEC.md v1.2 — inherited specification (unchanged sections noted by reference).
- ADR-001 — Git OID as canonical event/node ID (unchanged).
- ADR-002 — Kind reduction rationale.
- ADR-003 — Topic handle scheme.
- ADR-004 — Migration strategy.
- ADR-005 — Cross-clone conflict resolution rationale (LWW for non-handle events, explicit
  error for handle conflicts, atomic push, display-only short index).
- RFC-0027 — Topic meta-thread (superseded by this draft; this draft promotes the meta-thread to
  a first-class entity rather than a thread variant, but in slimmed form).
- RFC-0030 — Thread ID scheme (extended: `t-` prefix replaces per-kind prefixes).
- RFC-0031 — 3-letter kind prefixes (deprecated by this draft).
