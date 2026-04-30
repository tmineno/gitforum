# git-forum Product Specification — 2.0

Version 2.0 — 2026-04-30
Status: **Authoritative**. Inherits from SPEC.md v1.2 except where explicitly overridden below.
Bound by `doc/spec/CORE-VALUE.md` — when this document conflicts with the
core value statement, this document is wrong and must be revised.

> This specification introduces three structural changes to the 1.x model:
> 1. **Kind reduction** — the four thread kinds (`rfc`, `dec`, `task`, `issue`) collapse into a
>    single `thread` entity carried by `lifecycle` + free-form `tags`. The four 1.x kinds remain
>    as **stable CLI presets** (`new rfc`, `new task`, `new bug`, `new dec`) — the muscle memory
>    is preserved indefinitely; only the underlying schema changes.
> 2. **Node type reduction** — the ten 1.x node types collapse to four, cut by *protocol
>    effect* rather than rhetorical move: `comment`, `approval`, `objection`, `action`. The
>    standalone Approval concept (SPEC.md §2.7) folds into the node namespace.
> 3. **Topic as named context** — a new `topic` entity provides a memorable handle for
>    grouping related threads. **Threads remain the primary unit of work**; topics are
>    optional context wrappers, not a required ceremony layer. Standalone threads (no topic)
>    are first-class throughout the CLI, TUI, and default views.
>
> The topic concept in 2.0 is intentionally **slim**: a named container with an optional
> charter, and an archive flag. There is no topic state machine, no topic-level guards, and
> no nesting in 2.0. These capabilities are explicitly deferred to future minor releases (see
> Appendix A.3 for the forward-compatibility plan).
>
> **Distribution is not git-forum's job.** Forum data lives in `refs/forum/*` Git refs;
> users replicate it across clones with standard `git push` / `git fetch` on those refs.
> git-forum does not introduce its own push/fetch protocol or cross-clone conflict
> resolution. This is mandated by `CORE-VALUE.md`.
>
> The motivating analysis is recorded separately in ADR-002 (kind reduction), ADR-003 (topic
> handles), ADR-004 (migration), and ADR-006 (node type reduction). This document
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

Standalone threads (no topic) are permitted (see §2.2.2).

#### 2.1.3 Topic handle in references

Within a topic context, child threads may be referenced by **short index**:

```
!payment-rewrite          # the topic itself
!payment-rewrite/3        # the 3rd attached thread (display order)
```

The `/N` short index is a **display-only convenience**, not an identifier:

- It is computed from locally-visible `topic_attach` events ordered by
  `(timestamp, actor_id, event_oid)` — see §8.2 for stability rules.
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

#### 2.2.1 ID surface change

Thread IDs in 2.0 drop the kind-named prefix entirely. Kind information moves to
`facets.lifecycle` and conventional `tags` (e.g. `bug`, `task`, `cross-cutting`); the ID itself
no longer encodes a category.

| Surface | 1.x | 2.0 |
|---|---|---|
| Display | `RFC-6m4kap23` (kind-prefixed) | `@6m4kap23` (`@` type marker, see §6.0) |
| Storage | `refs/forum/threads/RFC-6m4kap23` | `refs/forum/threads/6m4kap23` (bare token) |

Legacy 1.x IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, `JOB-...`, `DEC-...`) remain valid for reading and
referencing in migrated repos via the alias table (see §10.1). New thread allocation always
uses the bare-token / `@`-display form (§6.2).

#### 2.2.2 Standalone threads

Threads MAY exist without a topic. This is the natural form for:

- Bug reports captured quickly before triage
- Questions that don't yet belong to any workstream
- One-off observations

Standalone threads can be promoted into a topic at any time via `topic attach`. Standalone is
a legitimate steady state, not a fault to be cleaned up — many bug reports and decision records
never need a topic. `doctor` reports the standalone count under the "Untriaged" section
(§9.5) as informational signal, never as a warning, and there is no inactivity threshold or
"becomes orphan after N days" rule. Users curate threads into topics when grouping helps and
otherwise leave them in the inbox.

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
  the node level (a `comment` whose body states the decision; see §2.5 / ADR-006) inside
  whatever thread reached that decision.
- `question` — questions are predominantly node-level inside other threads (also conveyed
  in `comment` body prose post-reduction).
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

#### 2.3.5 Tag grammar

Every tag MUST satisfy:

- ASCII lowercase only, `[a-z0-9-]` (no spaces, slashes, colons, `/`, `:`, `@`, `!`).
- Starts with a letter (`[a-z]`).
- Length 2–32 characters.
- Not equal to a reserved literal (`all`, `none`, `any`, `untagged`, `archived` — used as
  filter shorthands in `ls`/search, §9.3).

Violations are rejected at write time with `InvalidTagSyntax` (§13). The grammar is intentionally
narrow so tags compose cleanly with shell, search filters (`tag:bug`), and policy keys
(`creation_rules.execution.tag.bug`).

The 2.0 release ships **no tag registry, no conventional-tag list, no
unknown-tag warning, no deprecation surfacing, and no policy lint over
tag vocabulary**. Earlier drafts of this section specified a `.forum/
tags.toml` registry plus `UnknownTag` / `UnknownPolicyTag` /
`TagDeprecated` diagnostics; those mechanisms have been removed because
the language-drift problem they would solve has not been observed in
dogfood. Tag-vocabulary discipline is deferred to a future minor
release, gated on documented evidence of drift (per Appendix A.3 trigger
discipline).

The three conventional tag values used by the kind presets (`bug`,
`task`, `cross-cutting`; §9.2) are still produced by the presets, but
they are not preregistered anywhere — they are simply the strings the
preset emits.

### 2.4 Event

Unchanged from SPEC.md §2.3. New event types added in 2.0:

| Event type | Purpose | Payload (JSON shape; required fields shown) |
|---|---|---|
| `topic_create` | Initialize a topic (recorded on topic ref) | `{ "topic_id": <topic-id>, "title": <string>, "body"?: <string> }` |
| `topic_archive` | Mark topic as archived (sets `archived_at`) | `{ "at": <iso8601> }` |
| `topic_unarchive` | Reverse archive (clears `archived_at`) | `{}` |
| `topic_attach` | Bind a thread to a topic (recorded on the thread ref) | `{ "topic_id": <topic-id> }` |
| `topic_detach` | Remove a thread from a topic | `{}` |
| `topic_alias` | Add (default) or remove a topic alias | `{ "slug": <slug>, "op": "add" \| "remove" }` |
| `facet_set` | Change a thread's facet values (audited; see §7.3) | `{ "lifecycle"?: <string>, "tags_add"?: [<string>...], "tags_remove"?: [<string>...] }` |

#### 2.4.1 `facet_set` payload semantics

`facet_set` is a per-event mutation, not a full-state replacement. Replay rules:

- **`lifecycle`** — present only on the thread's first `facet_set` event (the implicit one
  written at thread creation), or never. §7.3 makes lifecycle immutable, so any subsequent
  `facet_set` carrying `lifecycle` MUST be rejected at write time with
  `FacetTransitionDisallowed`. Replay computes `lifecycle` as the value from the first event
  that carries it.
- **`tags_add` / `tags_remove`** — each event mutates the derived tag set. Replay walks the
  thread's event chain in `(timestamp, actor_id, event_oid)` order; for each event, every tag
  in `tags_add` is inserted into the set, then every tag in `tags_remove` is removed. Within
  a single event, `tags_add` is applied before `tags_remove` so an event that simultaneously
  adds and removes the same tag is a removal (rare; allowed for symmetry, not a useful
  pattern).
- Replay is purely append-order over the locally-visible event chain. There is no bespoke
  per-tag LWW reconciliation across clones; cross-clone tag merging follows whatever
  ordering Git presents after fetch, the same way any other event ordering does (§8.3).
- An empty `facet_set` payload (no `lifecycle`, no tag arrays) is valid and a no-op (allowed
  for backfill / hook purposes).

There is intentionally no `topic_state` event in 2.0. If a richer topic lifecycle is added
later (F-W1), it will be introduced as a new additive event without breaking topics created
under 2.0.

### 2.5 Node

**Overrides SPEC.md §4.3.** The 1.x ten-type set is reduced to four types,
cut by *protocol effect* rather than rhetorical move. See ADR-006 for
the rationale.

| Node type | Protocol effect |
|---|---|
| `comment` | None — body-prose contribution. Replaces 1.x `claim` / `question` / `summary` / `risk` / `review` / `alternative` / `assumption`. |
| `approval` | Positive — counts toward state-transition guards (e.g. `one_human_approval`). Folds in the standalone Approval concept from SPEC.md §2.7 (see §2.8). |
| `objection` | Negative — blocks state transitions until `resolve`d. Unchanged from 1.x. |
| `action` | Obligation — creates a tracked work item that must be `resolve`d before terminal states. Unchanged from 1.x. |

`evidence` remains a first-class non-node concept attached via
`evidence add` (§2.6); it is intentionally outside the node taxonomy.

Recording a decision is no longer a typed node. A decision is captured
as a `comment` whose body contains the decision text (and whose author
typically appends an `approval` once the decision is concluded). There
is no thread-level `decision` facet (see §2.3.4) and no `summary` node
type.

### 2.6 Evidence

Unchanged from SPEC.md §4.4.

### 2.7 Actor

Unchanged from SPEC.md §2.6.

### 2.8 Approval

The standalone Approval concept from SPEC.md §2.7 is folded into the
node namespace (§2.5). An approval is an `approval` node event, not a
separate event kind. The `--approve <actor>` flag on state-change
commands (§9.4) is preserved as a shortcut: it appends an `approval`
node and applies the state change in a single CLI invocation. Policy
guards (e.g. `one_human_approval`) key off `approval` nodes uniformly.

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
| `has-open` | One or more child threads in a non-terminal state |
| `all-terminal` | All child threads in a terminal state (`done`, `rejected`, `deprecated`, `withdrawn` — see §3.2) |

A topic whose only attached thread is `withdrawn` reports `all-terminal`, not `has-open` —
withdrawal is a deliberate end-state, not parking.

The richer red/yellow/green health model is deferred to F-W3 (Appendix A.3).

### 3.2 Thread state machine (unified)

A single state set with a deliberately permissive transition graph replaces the four 1.x
machines. Per-lifecycle restrictions are applied as a filter (§3.2.1) — the global graph below
contains every edge any lifecycle might need; the filter chooses which edges are reachable for
a given thread.

```text
draft -> open
draft -> withdrawn
open  -> working
open  -> review            # bypass `working` for proposals (RFC: draft -> open -> review -> done)
open  -> done              # bypass `working`/`review` for records (DEC: open -> done) and trivial bug closes
open  -> rejected
open  -> withdrawn
working -> review
working -> done            # bypass `review` for execution work that doesn't need formal review (bug fix landed, task complete)
working -> rejected
review  -> done
review  -> working
review  -> rejected
done    -> open            # reopen
rejected -> open
done    -> deprecated
rejected -> deprecated
```

Terminal states for the purposes of `topic show` summary (§3.1.1) and search filtering:
`done`, `rejected`, `deprecated`, `withdrawn`. No outgoing edges from `withdrawn` or
`deprecated` — both are absorbing terminals.

Initial state: depends on `lifecycle` (see §3.2.1).

#### 3.2.1 Lifecycle-filtered allowed states

The unified machine §3.2 is filtered by the thread's `lifecycle` facet. An edge is reachable
only if its destination state is in the lifecycle's allowed set:

| `lifecycle` | Allowed states | Initial | Typical path | Notes |
|---|---|---|---|---|
| `proposal` | `draft`, `open`, `review`, `done`, `rejected`, `withdrawn`, `deprecated` | `draft` | `draft → open → review → done` | `working` excluded — proposals don't have a "doing the work" state; that belongs to attached execution threads. `done` is the equivalent of 1.x `accepted` for RFCs. |
| `execution` | `open`, `working`, `review`, `done`, `rejected`, `deprecated` | `open` | bug: `open → done` (or `open → working → done`); task: `open → working → review → done` | All four edges out of `open`/`working` to `done`/`review` are available; the project's policy decides which is required for which tag (§7.2). `done` is the equivalent of 1.x `closed`. |
| `record` | `open`, `done`, `rejected`, `deprecated` | `open` | `open → done` | Records are short-lived; `working`/`review` excluded entirely. |

A transition whose destination is not in the lifecycle's allowed set is rejected with
`LifecycleStateMismatch` (§13). The error message names the lifecycle, the rejected state, and
the lifecycle's allowed-state list so the user can pick a valid alternative.

Terminal states (no outgoing edges in the global graph): `withdrawn`, `deprecated`. Edges to
`done` / `rejected` / `deprecated` are present in the global graph but their reachability per
lifecycle is determined by the table above.

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

- **Node types**: see §2.5 (overrides SPEC.md §4.3 — reduced to 4 types).
- **Evidence**: unchanged from SPEC.md §4.4.
- **Approval**: see §2.8 (folded into the node namespace; SPEC.md §4.5's standalone
  Approval event kind no longer exists).

## 5. Storage layout

### 5.1 Git refs

Authoritative data in 2.0:

```text
refs/forum/topics/<topic-id>      # topic event chain (NEW)
refs/forum/threads/<thread-id>    # thread event chain (unchanged structure)
refs/forum/aliases/<slug>         # alias-marker ref pointing at the current owner topic (NEW)
```

All ref-name segments are the **storage tokens** — bare alphanumeric (with `-` allowed in slug),
no `!` and no `@`. The user-facing markers (§6.0) are display-only and are stripped before the
ref name is constructed.

#### 5.1.1 Alias ref representation

Each `refs/forum/aliases/<slug>` ref is an **ordinary Git ref** (not a symref, not a note)
pointing at a **marker commit object**. The choice of "ordinary ref" is deliberate: it
makes alias refs work with standard `git push` / `git fetch` on the `refs/forum/*`
namespace, with no notes-fetch refspec gymnastics and no symref propagation quirks.

The marker commit has:

- An **empty tree** (`4b825dc...`, the canonical empty-tree OID).
- A **structured commit message** in trailer form:
  ```
  topic-alias <topic-id>

  slug: <slug>
  op: add | remove
  by: <actor-id>
  at: <iso8601>
  ```
- A **parent** equal to the previous tip of the same alias ref, when one exists. For a fresh
  `add` (slug previously unused), there is no parent. For a `remove` (rare; aliases never
  expire normally), the marker records `op: remove` and the resolver treats the slug as
  unbound from that commit forward.

The alias ref's own commit history therefore records the slug's binding history. A reader
walking `refs/forum/aliases/payment-rewrite` can reconstruct every claim and rename of the
slug, in addition to learning the current owner from the tip commit's `topic-alias` line.

#### 5.1.2 Handle resolution

Topic handle resolution walks `refs/forum/aliases/<slug>` first:

1. If the ref exists, parse the tip commit's `topic-alias <topic-id>` line. If the most recent
   `op` is `add`, the resolved owner is `<topic-id>`. If `remove`, the slug is currently
   unbound (resolver returns `TopicNotFound`).
2. If the ref does not exist, treat the input as a possible topic ID and look up
   `refs/forum/topics/<input>`.

### 5.2 Repository files

Same as SPEC.md §5.2 with added templates:

```text
.forum/
  policy.toml
  actors.toml
  templates/
    topic.md            # topic charter template (NEW)
    thread.md           # generic thread template (NEW)
    proposal.md         # preset for lifecycle=proposal (replaces rfc.md)
    execution.md        # preset for lifecycle=execution (replaces task.md / issue.md)
    record.md           # preset for lifecycle=record   (replaces dec.md)
```

Old per-kind templates (`rfc.md`, `issue.md`, etc.) are deprecated but readable for migration.

There is no `.forum/tags.toml` in 2.0 — tag-vocabulary discipline (registry,
conventional-tag list, deprecation, lint) is deferred per §2.3.5.

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
  §6.0.1 below. The `!` is **mandatory in machine-interpreted persisted references**
  (evidence refs, link targets, the `commit-msg` hook's structured ref scan) where type
  disambiguation matters. Free-form prose — body text, charter, comment-node bodies — is
  **not scanned**; users may write `!foo` or `foo` in prose without producing or violating a
  marker rule. The persisted-context check fires only at structured slots that the system
  itself parses as references (see §9.6).

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
| Positional or flag value where a thread is required (`thread show`, `thread state`, `comment`, `objection`, `action`, `evidence add`, etc.) | optional | `@` may be omitted for the same reason. `thread show a3f9b2k1` is equivalent to `thread show @a3f9b2k1`. |
| Mixed positions where either a topic or a thread is acceptable (e.g. `git forum show <REF>`) | **required** | Without the marker, the parser cannot disambiguate. Missing-marker input here returns `AmbiguousReferenceWithoutMarker` (§13) listing both candidate types. |
| Anywhere a reference is **structurally** persisted (evidence refs, link targets, the `commit-msg` hook's structured scan) | **required** | The persisted-context rule (§9.6) is unchanged: bare tokens at these slots are rejected as ambiguous. **Free-form body text, charter, and comment-node body prose are explicitly out of scope** — prose is not parsed for references. |

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

User MAY override with `--handle !pay`. The override is validated against the handle format
and locally checked for collision. Within-clone petname appending also applies to overridden
handles.

Cross-clone handle collisions (two clones independently claim the same slug) are detected by
standard Git push semantics: the second pusher's `refs/forum/aliases/<slug>` push fails as a
non-fast-forward, the way any other Git ref push fails. git-forum does not interpret this
failure or rewrite the handle automatically; the user resolves it by `topic rename` and
re-pushes, or by accepting whichever value Git fast-forwards to during fetch.

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

Within a known topic context, `<handle>/<N>` references the Nth thread attached to the topic
(1-indexed by `topic_attach` event order). Examples:

```
git forum show '!payment-rewrite/3'
```

Short references resolve to canonical thread IDs at parse time. They MUST NOT be stored as
canonical references in events or evidence (only canonical thread IDs are stored), and they
are rejected with `ShortIndexInPersistedRef` at every persisted entry point — see §8.3 and §9.6.

Named role labels (e.g. `!foo/design`) are reserved syntactically but **not specified** in 2.0
(see §2.1.3). The slash separator is used exclusively for numeric short index in 2.0.

### 6.4 Canonical event/node IDs

Unchanged from SPEC.md §6.2 (Git commit OID).

## 7. Policy

### 7.1 Facet-scoped guards

Guard rules in 2.0 are scoped by **facet expression** instead of kind:

```toml
# 2.0: facet-scoped
[[guards]]
on = "lifecycle=proposal AND tag=cross-cutting : review->done"
requires = ["one_human_approval", "no_open_objections"]

# 1.x equivalent (compat alias, internally rewritten):
[[guards]]
on = "rfc:under-review->accepted"
requires = ["one_human_approval", "no_open_objections"]
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

When a thread carries **multiple tags that each match a `tag.<name>` rule**, the tied-specificity
rules are merged with **field-level union** semantics: each field is the union (or stricter
choice) of all matching rules. Concretely:

| Field | Combiner |
|---|---|
| `required_body` | `OR` (any matching rule requiring a body wins) |
| `body_sections` | union of section names, deduplicated, preserving first-seen order |
| `requires` (guard predicates) | union of required predicates |
| numeric thresholds (e.g. `min_approvals`) | `MAX` |
| boolean strict-flags | `OR` (any `true` wins) |

This makes multi-tag policy compositional: tagging a thread `task,bug` enforces the union of
`tag.task` and `tag.bug` requirements rather than picking one arbitrarily. Users who want a
single rule to win can express the precedence explicitly with a guard predicate
(`tag=task AND NOT tag=bug`).

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

git-forum 2.0 operates only on the **local clone**. Cross-clone state
convergence is delegated to standard `git push` / `git fetch` on the
`refs/forum/*` namespace. git-forum does not introduce its own
distribution protocol, conflict-resolution algorithm, or atomic-push
group definition. This is mandated by `CORE-VALUE.md`.

When two clones converge via Git, the rules below apply locally on
each side and the diverging refs are reconciled by Git's standard
CAS / fast-forward / non-fast-forward semantics. Any non-fast-forward
push fails the way any other Git push fails; the user resolves it the
way they resolve any other Git divergence. Earlier drafts of this
section specified bespoke handle-conflict / attach-conflict / tag-LWW
protocols; those are removed.

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

Conflict cases (concurrent writes that touch overlapping state) fail at
the local CAS layer and are surfaced to the caller as a write failure.
Resolution is by re-reading and re-writing — the same retry pattern as
1.x. Cross-clone divergence (e.g. two clones independently attached
the same thread to different topics, then both pushed) is left to the
user to reconcile via Git tooling; `doctor` (§9.5) reports observed
divergence informationally.

### 8.2 Short-index stability

Topic short indices (`!foo/3`, §2.1.3) are **derived, session-local references**,
not canonical IDs:

- The mapping is computed at query time from locally-visible `topic_attach` events sorted by
  `(attach_event.timestamp, actor_id, event_oid)`.
- The mapping is local to the clone. After a fetch from a remote that introduces or
  reorders attach events, `/N` values may shift; users who care about a specific thread
  should reference it by canonical ID (`@<token>`).
- `/N` MUST NOT appear in stored data: not in evidence refs, not in link targets, not in commit
  messages used by hooks. Only canonical thread IDs (`@XXXXXXXX`) and topic handles are
  stored.
- Implementations **MUST reject** `/N` references at every persisted entry point with
  `ShortIndexInPersistedRef` (an error in 2.0 — see §13 and the entry-point table in §9.6).
  This is a hard error, not a warning, in 2.0; treating it as advisory was rejected because
  the failure mode it prevents (a stored short-ref silently meaning a different thread on
  another clone) is a correctness issue, not a stylistic one.

### 8.3 Distribution

Forum data is replicated between clones with standard `git push` and
`git fetch` on the `refs/forum/*` namespace. git-forum does not wrap
these commands and does not introduce its own push/fetch protocol.
The atomic-push and recommended-ordering guidance from earlier drafts
of this section has been removed; users follow whatever Git fetch/push
workflow they already use for code refs.

When a non-fast-forward push fails, the user resolves it with the
standard Git workflow (fetch, rebase or merge their forum refs,
re-push). git-forum does not assume responsibility for the merge
strategy. `git forum doctor` (§9.5) reports any divergence visible in
the local refs (e.g., a thread attached to two different topics in
two different ancestor commits) so the user knows what needs manual
attention.

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

Because git-forum delegates push/fetch to standard Git (§8.3), publication-state lines (e.g.
"local only", "handle not pushed") are not part of the header. Whether a topic has been
pushed to a remote is a question for `git for-each-ref` / `git ls-remote`, the same way it
would be answered for any other ref namespace.

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

Inherits SPEC.md §9.4 / §9.5 / §9.7 / §9.10 with the **node-shorthand reduction** from
ADR-006 / §2.5:

| Canonical command | Shorthand | Status in 2.0 |
|---|---|---|
| `node add --type comment` | `comment` | new (replaces `claim` / `question` / `summary` / `risk` / `review`) |
| `node add --type objection` | `objection` | unchanged |
| `node add --type action` | `action` | unchanged |
| (state change with `--approve`) | `approve` | unchanged in form; emits an `approval` node (§2.8) instead of a separate Approval event |

`claim` / `question` / `summary` / `risk` / `review` shorthands are aliased to `comment` for
one minor release with a deprecation warning, then removed in 3.0. Authors who relied on
the rhetorical distinction express it in the body (e.g. start the comment with `Q:`,
`Decision:`, `Risk:`).

State-change shorthand commands (`close`, `accept`, etc.) continue to work and map to the
unified state machine via the thread's lifecycle facet:

| Shorthand | `lifecycle=execution` | `lifecycle=proposal` | `lifecycle=record` |
|---|---|---|---|
| `close` | → `done` | (rejected: use `accept`) | → `done` |
| `accept` | (rejected: use `close`) | → `done` | → `done` |
| `propose` | (rejected) | → `open` (from `draft`) | (rejected) |
| `pend` | → `working` | (rejected) | (rejected) |
| `reject` | → `rejected` | → `rejected` | → `rejected` |
| `withdraw` | (rejected: use `close` or `reject`) | → `withdrawn` (from `draft` or `open`) | (rejected) |
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
  - Broken aliases, dangling attach references, and any divergence visible in local refs
    after a fetch (e.g. a thread carrying two `topic_attach` events to different topics in
    different ancestor commits) — these *are* warnings, surfaced for the user to reconcile
    via plain Git tooling per §8.3.
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

- Existing thread refs are rewritten: `refs/forum/threads/RFC-0001` →
  `refs/forum/threads/<thread-id>` (storage form per §5.1 / §6.2; display form
  `@<thread-id>`). The old name is preserved as a read-only alias entry so external links
  (`RFC-0001`, `ASK-XXXXXXXX`, etc.) keep resolving.
- Each thread gets a `facet_set` event added to its history populating `lifecycle` and the
  conventional `tags` (`cross-cutting` for `rfc`; `bug` for `issue`; `task` for `task`) per the
  §2.3.3 mapping.
- States are remapped per §3.2.2.
- **Node events are rewritten** per ADR-006 / §2.5: 1.x types `claim` / `question` /
  `summary` / `risk` / `review` / `alternative` / `assumption` become `comment` (with
  `legacy_subtype` preserved); standalone Approval events become `approval` nodes.
  `objection`, `action`, and `evidence` are unchanged.
- **Migrated threads remain standalone** (no topic attachment). No `!_legacy` topic is
  auto-created. Users attach threads to topics manually as they triage. `doctor` reports the
  standalone count under the "Untriaged" section after migration as an informational signal
  — it will be high initially and is expected to decrease over time as triage proceeds.

Rationale for leaving threads standalone rather than auto-bucketing: a synthetic `!_legacy`
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
| `AttachToArchivedTopic` | **error** | attach attempt to a topic whose `archived_at` is set | `--force` overrides; intentional gate to keep work visible |
| `ShortIndexInPersistedRef` | **error** | `/N` short reference appears where it would be stored (e.g. commit message scanned by `commit-msg` hook, evidence ref, link target) | Error message MUST include the canonical thread ID resolved at the moment the short reference was rejected (e.g., "did you mean `@d8f4q9aa`?"). |
| `AmbiguousReferenceWithoutMarker` | error | Bare token (no `!`/`@`) used in a CLI position that accepts both a topic and a thread (e.g. `git forum show <REF>`) | Lists candidate topic / thread matches; suggests prefixing with `!` or `@` to disambiguate. |
| `InvalidTagSyntax` | error | `--tag <value>` or `facet_set` payload violates the tag grammar (§2.3.5) | Message names the offending character / length / reserved-literal violation; suggests a sanitized form. |

Cross-clone divergence (handle conflicts, attach conflicts, tag drift) is **not** surfaced
through dedicated error codes in 2.0 — it appears as ordinary Git push/fetch failures, the
same way any other ref divergence would. Tag-vocabulary diagnostics (`UnknownTag`,
`UnknownPolicyTag`, `TagDeprecated`) and the standalone-Approval error space are removed
along with the features they reported on (§2.3.5, §2.8, §8.3).

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

### Tag grammar (§2.3.5)

- `--tag` rejects values violating the grammar (uppercase, leading digit, length &lt;2 or
  &gt;32, contains `/`, `:`, `@`, `!`, space, reserved literals like
  `all`/`untagged`/`archived`) with `InvalidTagSyntax`. The error message names the specific
  violation and proposes a sanitized form.

### Node type reduction (ADR-006, §2.5)

- 1.x node events of types `claim` / `question` / `summary` / `risk` / `review` /
  `alternative` / `assumption` migrate to `comment` with the legacy type label preserved
  in `legacy_subtype`.
- 1.x standalone Approval events migrate to `approval` node events.
- Policy guards predicated on the old types resolve via the same legacy-subtype
  preservation; `at_least_one_summary` is no longer shipped as a guard predicate (§7.1).

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

- `/N` accepted as input to read-only CLI commands (`show`, etc.).
- `/N` **rejected with `ShortIndexInPersistedRef`** in every persisted-context check point
  enumerated in §9.6.
- The rejection message includes the canonical thread ID the short reference resolved to
  (e.g. `@d8f4q9aa`); when the topic itself does not resolve locally, the message says so
  explicitly instead of returning an opaque "unknown reference" error.

## 15. Non-goals

In addition to SPEC.md §15 and the five non-goals in `doc/spec/CORE-VALUE.md`:

- General-purpose project management (Gantt charts, dependency graphs across topics).
- Topic state machines, topic-level guards, topic nesting in 2.0
  (intentionally deferred — see Appendix A.3).
- Multi-parent topics (DAG of topics).
- User-defined required facet axes beyond `lifecycle` (use `tags` instead).
- Mandatory topic membership for threads.
- A `git forum push` / `git forum fetch` command, atomic-ref-group semantics, or any
  cross-clone conflict-resolution protocol. Distribution is plain Git on `refs/forum/*`
  (§8.3, CORE-VALUE.md non-goal §3).
- A tag registry, conventional-tag list, unknown-tag warnings, deprecation surfacing, or
  tag-vocabulary policy lint. Earlier drafts of 2.0 specified `.forum/tags.toml` and
  related diagnostics; these are removed in 2.0 and deferred per §2.3.5.

## Appendix A: Open questions

### A.1 Resolved during 2.0 drafting

| ID | Question | Resolution |
|---|---|---|
| O-1 | Should `!_legacy` migration bucket be created automatically, or should threads stay orphan until manually attached? | **Orphan**. No synthetic topic on migration; `doctor` reports orphan count. (§10.1) |
| O-2 | Are 5 intent values enough? | **Dropped entirely**, and `scope` was dropped too. Sole required facet is `lifecycle`; everything else (bug/task/cross-cutting) is a tag. (§2.3) |
| O-3 | Should standalone threads be allowed to use shorthand commands (`close`, `accept`) directly? | **Yes**. Shorthand commands work uniformly on standalone and attached threads. Topic attachment is never required for state changes. (§9.4) |
| O-4 | Should free-form tags have any constraint, given the language-drift risk (`bug` vs `defect` vs `issue`)? | **Grammar only.** Hard tag grammar (`[a-z][a-z0-9-]{1,31}`); no registry, no conventional-tag list, no unknown-tag diagnostic, no policy lint over tag vocabulary. Drift remediation is deferred per F-T1 (Appendix A.3) until dogfood evidence shows the grammar is insufficient. (§2.3.5) |
| O-5 | Should the ten 1.x node types be preserved, or reduced? | **Reduced to four** by protocol effect: `comment`, `approval`, `objection`, `action`. The standalone Approval concept folds into the `approval` node. See ADR-006 / §2.5. |
| O-6 | Should 2.0 ship a `git forum push` / `git forum fetch` and cross-clone conflict-resolution protocol? | **No.** Distribution is delegated to plain Git on `refs/forum/*`. CORE-VALUE.md non-goal §3 forbids reinventing the protocol. (§8.3) |

### A.2 Remaining for 2.0 implementation

(none currently outstanding — to be added as implementation surfaces design questions)

### A.3 Deferred from Level XS scoping (forward-compatibility plan)

The following capabilities were considered for 2.0 and **deliberately deferred** to keep the
release scope tight. Each can be added in a 2.x minor release without breaking 2.0 clients,
provided the additive contracts below are honored.

| ID | Capability | Current 2.0 substitute | Trigger to add | Forward-compat contract |
|---|---|---|---|---|
| F-W1 | Topic state machine (e.g. `planning` / `active` / `wrapping` / `done` / `abandoned`) | `archived_at` flag + derived summary | Need to express stage of work as a queryable signal beyond "active vs archived" | Introduce `topic_state` event type (additive). Topics without any `topic_state` event default to `active`. `archived` remains derived from `archived_at` and is orthogonal to status. **Note: per CORE-VALUE non-goal §1, this MUST NOT introduce cross-thread workflow enforcement.** |
| F-W2 | Topic-level guards | None (rely on per-thread guards) | Need to enforce conditions on topic archival or future state transitions (e.g. "all children terminal before archive") | Add `[[topic_guards]]` policy section. Guards on `unrestricted` operations (archive in 2.0) are absent by default; adding rules later affects only repos that opt in. **Note: same constraint as F-W1 — cross-thread coupling crosses the CORE-VALUE line.** |
| F-W3 | Richer derived health (`green` / `yellow` / `red`) replacing the simple summary | `empty` / `has-open` / `all-terminal` | Need to surface "stuck" topics visually (e.g. unresolved objections, stale activity) | Health is a pure function of child state. Richer logic adds without breaking simpler clients; index columns gain a `topic_health` field, summary remains for backward queries. |
| F-W4 | Topic nesting (single-parent) | Flat topics only | Need to express epic / sub-topic hierarchy | Add optional `parent` field to topics. Absent = root. Cycles rejected at write time. Existing 2.0 topics are roots by default. |
| F-T1 | Tag-vocabulary discipline (registry, conventional list, deprecation, lint) | None — bare grammar only (§2.3.5) | Documented language drift across clones (`bug` vs `defect`) producing search/policy split | Re-introduce `.forum/tags.toml` with the schema described in earlier 2.0 drafts (`description`, `aliases`, `deprecated`, `replaced_by`). All write paths emit warnings only by default; strict mode is opt-in. |

#### Why Level XS over Level XXS

Level XXS would have collapsed `topic` to a tag-like string field on threads, eliminating the
topic ref tree entirely. That model is simpler still, but it would have made F-W1 and F-W4
**re-architecture** changes (introducing a new entity), not additive ones. Level XS preserves
topic as a first-class entity so all four future capabilities above remain forward-compatible.

#### Trigger discipline

A future minor release SHOULD add a deferred capability only when:

1. Documented dogfood evidence shows the substitute is insufficient.
2. The additive contract above is honored (no breaking change for clients on prior minor).
3. The corresponding ADR is written and accepted.

Speculative implementation of F-W1–F-W4 / F-T1 without these triggers is explicitly discouraged.

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

$ git forum comment @a3f9 "Resize handler doesn't account for negative width on shrink"
appended comment node n-5h2m9p1k

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
    --topic payment-system-rewrite --edit          # `--topic` accepts the bare slug; the `!` is display-only
created thread @x9k2m4p7
  lifecycle:  proposal
  tags:       cross-cutting
  status:     draft
  topic:   !payment-system-rewrite (slot /1)

$ git forum comment   @x9k2 "Q: How do we handle ordering invariants in the queue?"
$ git forum objection @x9k2 "Async retries can violate at-most-once delivery"

# After review:
$ git forum resolve @x9k2 n-9b3c4d5e
$ git forum comment @x9k2 "Decision: queue-based dispatch with idempotency keys"
$ git forum propose @x9k2          # draft -> open
$ git forum state   @x9k2 review   # open -> review
$ git forum accept  @x9k2 --approve human/alice
state: review -> done
```

`--approve human/alice` appends an `approval` node and applies the state change in a single
event (§2.8); it satisfies the `one_human_approval` predicate of the
`lifecycle=proposal AND tag=cross-cutting : review->done` guard from §7.1. Rhetorical
distinctions ("Q:", "Decision:") are conveyed in the comment body, not via separate node
types (ADR-006).

### B.3 Implementation task linked to the RFC

```text
$ git forum new task "Implement async queue dispatcher" \
    --topic payment-system-rewrite \
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

$ git forum show '!payment-system-rewrite'      # mixed-position command: marker required + quoted to defeat `!` history expansion
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
$ git forum topic attach payment-system-rewrite @a3f9b2     # topic-typed slot: marker optional
attached @a3f9b2 to !payment-system-rewrite (slot /4)
```

The thread is no longer standalone; it now appears in the topic's `show` output.

### B.7 Tag-driven policy customization

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
requires = ["one_human_approval", "no_open_objections"]
```

When a thread tagged `task` is created without acceptance criteria, the operation check fires a
warning (or error if `strict = true`). The 1.x `at_least_one_summary` predicate is no longer
shipped (ADR-006 removed `summary` as a node type); maintainers who want forced summaries
can require a body section via `body_sections`.

## Appendix C: References

- `doc/spec/CORE-VALUE.md` — upstream constraint document; bounds this specification.
- SPEC.md v1.2 — inherited specification (unchanged sections noted by reference).
- ADR-001 — Git OID as canonical event/node ID (unchanged).
- ADR-002 — Kind reduction rationale.
- ADR-003 — Topic handle scheme.
- ADR-004 — Migration strategy.
- ADR-006 — Node type reduction (collapses 10 types to 4 by protocol effect).
- (ADR-005 — cross-clone conflict resolution — was removed when distribution was
  delegated to plain Git; see §8.3 and CORE-VALUE.md non-goal §3.)
- RFC-0027 — Topic meta-thread (superseded by this draft; this draft promotes the meta-thread to
  a first-class entity rather than a thread variant, but in slimmed form, and explicitly
  rejects the cross-thread workflow enforcement that motivated RFC-0027).
- RFC-0030 — Thread ID scheme (extended: kind-named prefixes drop entirely; the `@` type
  marker becomes the display form per §6.0 and §6.2; storage is the bare 8-char token).
- RFC-0031 — 3-letter kind prefixes (deprecated by this draft).
