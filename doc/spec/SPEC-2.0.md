# git-forum Product Specification — 2.0

Version 2.0 — 2026-04-30
Status: **Authoritative**. Inherits from SPEC.md v1.2 except where explicitly overridden below.
Bound by `doc/spec/CORE-VALUE.md` — when this document conflicts with the
core value statement, this document is wrong and must be revised.

> This specification introduces two structural changes to the 1.x model:
> 1. **Kind reduction** — the four thread kinds (`rfc`, `dec`, `task`, `issue`) collapse into a
>    single `thread` entity carried by `lifecycle` + free-form `tags`. The four 1.x kinds remain
>    as **stable CLI presets** (`new rfc`, `new task`, `new bug`, `new dec`) — the muscle memory
>    is preserved indefinitely; only the underlying schema changes.
> 2. **Node type reduction** — the ten 1.x node types collapse to four, cut by *protocol
>    effect* rather than rhetorical move: `comment`, `approval`, `objection`, `action`. The
>    standalone Approval concept (SPEC.md §2.7) folds into the node namespace.
>
> Earlier 2.0 drafts also introduced a *topic* entity for grouping
> related threads under a memorable handle. That mechanism has been
> removed. Empirically the grouping users wanted is "an RFC plus the
> threads that link to it via `--rel implements` / `--rel relates-to`" —
> something the existing thread-link relations already express. Display
> the group via advisory output (`git forum show <parent>` lists its
> linked children, see CORE-VALUE.md "Advisories"); no separate topic
> entity, ref tree, alias scheme, or `!` symbol is required.
>
> **Distribution is not git-forum's job.** Forum data lives in `refs/forum/*` Git refs;
> users replicate it across clones with standard `git push` / `git fetch` on those refs.
> git-forum does not introduce its own push/fetch protocol or cross-clone conflict
> resolution. This is mandated by `CORE-VALUE.md`.
>
> The motivating analysis is recorded separately in ADR-002 (kind reduction), ADR-004
> (migration), and ADR-006 (node type reduction). This document specifies the resulting
> model.

## 1. Overview

### 1.1 What changes versus 1.x

| Concern | 1.x | 2.0 |
|---|---|---|
| Primary unit of work | Thread (`RFC-...`, `JOB-...`) | **Thread** (unchanged). |
| Thread classification | `kind` enum: `rfc` / `dec` / `task` / `issue` | **Single required facet** (`lifecycle`) + free-form `tags` |
| State machines | 4 kind-specific machines | 1 unified machine, allowed states gated by `lifecycle` facet |
| Node types | 10 types (claim, question, ...) | 4 types: `comment`, `approval`, `objection`, `action` (ADR-006) |
| Top-level CLI | `git forum new rfc ...` etc. | `git forum new rfc/task/bug/dec ...` remain as the **stable everyday surface**; `git forum thread new --lifecycle ...` is the canonical/scriptable form |
| Thread grouping | Links between threads (`--link-to ... --rel ...`) | Unchanged. The "group" is a parent thread + its linked children, surfaced as an advisory in `show` / `verify`. No separate topic entity. |

### 1.2 Design principles (additions to 1.x)

In addition to the six principles in SPEC.md §1.1, 2.0 adds:

7. **Composable taxonomy.** Thread classification is built from independent facets, not enumerated
   kinds. New use cases extend the facet vocabulary, not the kind set.
8. **Quick-capture-first.** A short bug report or note must take seconds, not minutes. Stable
   kind presets (`new bug`, `new task`, `new rfc`, `new dec`) keep the friction low for common
   cases.

### 1.3 Implementation constraints

Unchanged from SPEC.md §1.2.

## 2. Core model

### 2.1 Thread grouping (links, not topics)

Threads in 2.0 are grouped via the **link relations** that already exist in 1.x
(`--rel implements`, `--rel relates-to`, `--rel depends-on`, `--rel blocks`,
`--rel supersedes`, etc.). The "group" associated with a parent thread `P` is the
set of threads transitively reachable from `P` through these relations; there is
no separate topic entity.

Earlier 2.0 drafts introduced a `topic` entity with handles (`!payment-rewrite`),
alias refs, attach/detach events, and a topic-scoped short-index (`!foo/3`).
That mechanism has been removed:

- The dogfood-observed grouping need ("the RFC and everything implementing it")
  is already expressible with `--rel implements` and friends.
- A separate handle namespace, ref tree, and event family added implementation
  surface and a markup symbol (`!`) for value already obtainable from
  link-tree advisories.
- Per CORE-VALUE.md, advisory cross-thread display (e.g. `git forum show <RFC>`
  listing its linked children with their states) covers the visualization
  need without a new entity.

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
| `scope.branch` | string | Bound Git branch |
| `links[]` | array | Thread-to-thread links (the only grouping mechanism in 2.0; see §2.1) |

#### 2.2.1 ID surface change

Thread IDs in 2.0 drop the kind-named prefix entirely. Kind information moves to
`facets.lifecycle` and conventional `tags` (e.g. `bug`, `task`, `cross-cutting`); the ID itself
no longer encodes a category.

| Surface | 1.x | 2.0 |
|---|---|---|
| Display | `RFC-6m4kap23` (kind-prefixed) | `@6m4kap23` (`@` type marker, see §6.1) |
| Storage | `refs/forum/threads/RFC-6m4kap23` | `refs/forum/threads/6m4kap23` (bare token) |

Legacy 1.x IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, `JOB-...`, `DEC-...`) remain valid for reading and
referencing in migrated repos via the alias table (see §10.1). New thread allocation always
uses the bare-token / `@`-display form (§6.2).

### 2.3 Facets

A thread's classification is **one required facet** plus free-form tags.

#### 2.3.1 Required facet

| Facet | Values | Meaning |
|---|---|---|
| `lifecycle` | `proposal` / `execution` / `record` | How the thread progresses (gates the state machine) |

`lifecycle` is the only required facet because it is the only one the **state machine itself**
depends on (§3.1.1). Everything else — bug-vs-task, cross-cutting-vs-local, sub-team routing — is a
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

Three tag strings are emitted by the kind presets (§9.1):

| Tag | Conventional meaning | Emitted by |
|---|---|---|
| `bug` | Observation-style execution thread (legacy `ISSUE` / `ASK`) | `git forum new bug` / `new issue` |
| `task` | Work-style execution thread (legacy `TASK` / `JOB`) | `git forum new task` |
| `cross-cutting` | Wide-impact thread (legacy `RFC` carries this by convention) | `git forum new rfc` |

These are convention only — they are not pre-registered anywhere (the registry was removed
in 2.0; see §2.3.5). Nothing in the core model depends on these specific values; repos that
prefer a different vocabulary use `git forum thread new --tag <other>` directly.

#### 2.3.3 Mapping from 1.x kinds

The 1.x four-kind taxonomy maps to 2.0 as follows:

| 1.x kind | lifecycle | conventional tags |
|---|---|---|
| `rfc` | `proposal` | `cross-cutting` |
| `dec` | `record` | (none) |
| `task` (`JOB`) | `execution` | `task` |
| `issue` (`ASK`) | `execution` | `bug` |

These four combinations are exposed as **kind presets** (compatibility shorthands; §9.1).

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
  filter shorthands in `ls`/search, §9.2).

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
`task`, `cross-cutting`; §9.1) are still produced by the presets, but
they are not preregistered anywhere — they are simply the strings the
preset emits.

### 2.4 Event

Unchanged from SPEC.md §2.3. One new event type added in 2.0:

| Event type | Purpose | Payload (JSON shape; required fields shown) |
|---|---|---|
| `facet_set` | Change a thread's facet values (audited; see §7.3) | `{ "lifecycle"?: <string>, "tags_add"?: [<string>...], "tags_remove"?: [<string>...] }` |

Earlier 2.0 drafts also added six topic event types (`topic_create`, `topic_archive`,
`topic_unarchive`, `topic_attach`, `topic_detach`, `topic_alias`). These are removed
along with the topic mechanism (§2.1).

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
  ordering Git presents after fetch, the same way any other event ordering does (§8.2).
- An empty `facet_set` payload (no `lifecycle`, no tag arrays) is valid and a no-op (allowed
  for backfill / hook purposes).

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
commands (§9.3) is preserved as a shortcut: it appends an `approval`
node and applies the state change in a single CLI invocation. Policy
guards (e.g. `one_human_approval`) key off `approval` nodes uniformly.

## 3. State machines

### 3.1 Thread state machine (unified)

A single state set with a deliberately permissive transition graph replaces the four 1.x
machines. Per-lifecycle restrictions are applied as a filter (§3.1.1) — the global graph below
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

Terminal states for search filtering: `done`, `rejected`, `deprecated`, `withdrawn`.
No outgoing edges from `withdrawn` or `deprecated` — both are absorbing terminals.

Initial state: depends on `lifecycle` (see §3.1.1).

#### 3.1.1 Lifecycle-filtered allowed states

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

#### 3.1.2 Mapping from 1.x states

Migration §10 specifies the 1.x → 2.0 state mapping. The mapping is lossless: every 1.x state has a
unique 2.0 equivalent.

### 3.2 State derivation

Unchanged from SPEC.md §3.5.

## 4. Data model

### 4.1 Thread

See §2.2 for fields. Field `kind` from 1.x is **removed**; replaced by `facets`.

### 4.2 Event

See §2.4. New event types listed; existing event types unchanged.

### 4.3 Node types, Evidence, Approval

- **Node types**: see §2.5 (overrides SPEC.md §4.3 — reduced to 4 types).
- **Evidence**: unchanged from SPEC.md §4.4.
- **Approval**: see §2.8 (folded into the node namespace; SPEC.md §4.5's standalone
  Approval event kind no longer exists).

## 5. Storage layout

### 5.1 Git refs

Authoritative data in 2.0:

```text
refs/forum/threads/<thread-id>    # thread event chain (unchanged structure from 1.x)
```

The 1.x layout is preserved: only one ref tree (`refs/forum/threads/<thread-id>`) is
authoritative. Earlier 2.0 drafts also defined `refs/forum/topics/<topic-id>` and
`refs/forum/aliases/<slug>` for the topic mechanism; with topics removed, those ref
trees do not exist in 2.0.

### 5.2 Repository files

Same as SPEC.md §5.2 with simplified templates:

```text
.forum/
  policy.toml
  actors.toml
  templates/
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

### 6.1 Type-marker symbol

User-facing thread identifiers carry a leading `@` type marker:

| Marker | Type | Storage form | Display form |
|---|---|---|---|
| `@` | thread ID | `<8-char-base36>` under `refs/forum/threads/<token>` | `@<token>` |

The earlier 2.0 draft also defined `!` for topic handles and `/` as a topic-scoped
short-index separator; both have been removed along with the topic mechanism (§2.1).

The `@` marker is **shell-safe** (no quoting needed), echoes the "at this address /
conversation point" meaning, and is preserved purely as a display-form prefix — refs, file
paths, and serialized event fields all use the bare token (Git ref-name validation reserves
`@{` syntax, so `@` itself is allowed as a display-only prefix that is stripped before ref
construction).

#### 6.1.1 Type-marker omission at CLI input

The CLI **MUST accept** thread references with the `@` omitted in every position. The
command grammar always knows whether a thread is expected (no other entity type exists in
2.0), so the marker carries no disambiguation load.

```
git forum show a3f9b2k1            # equivalent to: git forum show @a3f9b2k1
git forum thread state a3f9 review # equivalent to: git forum thread state @a3f9 review
```

The `@` remains the canonical display form in `show` / `ls` output and is **mandatory** in
machine-interpreted persisted references (evidence refs, link targets, the `commit-msg`
hook's structured ref scan) so future widening of the marker scheme remains
forward-compatible. Free-form prose — body text, comment-node bodies — is **not scanned**;
users may write `@foo` or `foo` without producing or violating a marker rule.

### 6.2 Thread IDs

**Display form**: `@XXXXXXXX` where `XXXXXXXX` is 8 base36 chars. Storage uses the bare
`XXXXXXXX` under `refs/forum/threads/`. Generation algorithm and collision analysis
identical to SPEC.md §6.1, but the kind-prefix machinery is replaced by the type symbol.

Legacy 1.x thread IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, etc.) remain valid for reading. The parser
accepts:

- `@XXXXXXXX` (2.0 native, display form)
- Bare `XXXXXXXX` (2.0 storage form, also accepted at CLI)
- Legacy `<KIND>-XXXXXXXX` (1.x opaque)
- Legacy `<KIND>-NNNN` (1.x sequential)

Unambiguous prefixes (≥4 chars after `@`) accepted as in 1.x.

### 6.3 Canonical event/node IDs

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

When a thread carries **multiple tags that each match a `tag.<name>` rule**, 2.0 keeps
the resolution intentionally minimal: rules MUST be expressed against a single tag
(`creation_rules.<lifecycle>.tag.<name>`) or against a guard predicate that itself
disambiguates (`tag=task AND NOT tag=bug`). Implementations MAY pick any matching
rule deterministically (e.g., first by alphabetical tag name) when a thread carries
multiple tags whose rules tie, but the spec does not mandate a per-field union /
intersection combiner. Multi-tag combiners (field-level union with explicit
`OR`/`MAX` semantics) are deferred until dogfood evidence shows the simple resolution
is insufficient.

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
- Concurrent writes to different threads are fully safe.
- Concurrent writes to the same thread fail with a conflict error; the caller retries.

#### 8.1.1 Semantic merge (extended for 2.0)

Auto-merge cases, in addition to those in SPEC.md §8.1:

- Concurrent `facet_set` events that change disjoint tag sets (additive merge).

Conflict cases (concurrent writes that touch overlapping state) fail at the local CAS layer
and are surfaced to the caller as a write failure. Resolution is by re-reading and
re-writing — the same retry pattern as 1.x. Cross-clone divergence is left to the user to
reconcile via Git tooling; `doctor` (§9.4) reports observed divergence informationally.

### 8.2 Distribution

Forum data is replicated between clones with standard `git push` and `git fetch` on the
`refs/forum/*` namespace. git-forum does not wrap these commands and does not introduce
its own push/fetch protocol.

When a non-fast-forward push fails, the user resolves it with the standard Git workflow
(fetch, rebase or merge their forum refs, re-push). git-forum does not assume responsibility
for the merge strategy. `git forum doctor` (§9.5) reports any divergence visible in the
local refs informationally.

## 9. CLI surface

### 9.1 Thread commands (unified + presets)

Canonical form:

```text
git forum thread new <TITLE>
    --lifecycle <LIFECYCLE>
    [--tag <TAG>...]
    [--body <TEXT> | --body-file <PATH> | --edit]
    [--branch <BRANCH>] [--link-to <THREAD> --rel <REL>]
    [--from-commit <REV>] [--from-thread <THREAD>] [--force]
git forum thread show <THREAD> [--tree]
git forum thread ls [--lifecycle <LIFECYCLE>]
    [--status <STATUS>] [--tag <TAG>] [--branch <BRANCH>]
git forum thread state <THREAD> <NEW_STATE> [--approve <ACTOR>]... [--comment <TEXT>]
git forum thread tag add <THREAD> <TAG>...
git forum thread tag rm  <THREAD> <TAG>...
```

`thread show --tree` is an **advisory** display: it lists the thread's linked children
(via `--rel implements` / `relates-to` / etc.) and their current states. It does not gate
any operation. See CORE-VALUE.md "Advisories" for the boundary against cross-thread
enforcement.

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

### 9.2 Listing and display

```text
git forum ls                                       # all threads, sorted by updated_at desc
git forum ls --lifecycle <LIFECYCLE>               # filter by lifecycle facet
git forum ls --tag <TAG>                           # filter by tag
git forum ls --status <STATUS>                     # filter by state
git forum show <THREAD>                            # show one thread (with --tree, list linked children)
```

`git forum ls` is a flat list. Earlier 2.0 drafts split the default view into "Topics" and
"Inbox" sections; with topics removed, the default is the simple flat list.

### 9.3 Discussion, lifecycle, evidence, links, hooks

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

### 9.4 Preflight, doctor

`git forum verify <THREAD>` and `git forum doctor` continue to work as in 1.x:

- `verify` is a single-thread guard preflight (no cross-thread enforcement).
- `verify` MAY surface an **advisory** noting the state of threads linked from the verified
  thread (e.g., "linked RFC `@1ooguji1` is not yet `done`"); this is informational only and
  does not block the verification result. See CORE-VALUE.md "Advisories".
- `doctor` reports broken refs, dangling link references, and any divergence visible in
  local refs after a fetch — surfaced for the user to reconcile via plain Git tooling
  (§8.2).

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
- States are remapped per §3.1.2.
- **Node events are rewritten** per ADR-006 / §2.5: 1.x types `claim` / `question` /
  `summary` / `risk` / `review` / `alternative` / `assumption` become `comment` (with
  `legacy_subtype` preserved); standalone Approval events become `approval` nodes.
  `objection`, `action`, and `evidence` are unchanged.
- Existing thread-to-thread links (`link` events with `--rel <REL>`) are preserved
  unchanged. They are the only grouping mechanism in 2.0 (§2.1).

### 10.2 What is permanent vs deprecated

**Permanent (no removal scheduled):**

- Top-level kind-named commands: `git forum new rfc/dec/task/issue/bug` and the corresponding
  `close` / `accept` / `pend` / `propose` / `reject` / `deprecate` shorthands. These are the
  stable everyday surface (§9.1).

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

Inherits SPEC.md §11. The 2.0 changes are:

- Thread detail header shows `lifecycle` and `tags` instead of `kind`.
- A thread-detail "linked" panel surfaces children (advisory, no enforcement) — see
  CORE-VALUE.md "Advisories".

No topic-related views: there are no topic, alias, or attach screens in 2.0.

## 12. Search

Search index gains a `lifecycle` column and a `tags` join table replacing the `kind`
column.

Old search queries referencing `kind:rfc` are auto-translated to
`lifecycle:proposal AND tag:cross-cutting` for one minor release. `kind:issue` translates to
`lifecycle:execution AND tag:bug`; `kind:task` to `lifecycle:execution AND tag:task`; `kind:dec`
to `lifecycle:record`.

## 13. Error handling

Unchanged from SPEC.md §13. New error and warning categories:

| Code | Severity | Triggered by | Notes |
|---|---|---|---|
| `FacetTransitionDisallowed` | error | facet mutation in a state that doesn't allow it | |
| `LifecycleStateMismatch` | error | state transition not allowed for thread's lifecycle | |
| `InvalidTagSyntax` | error | `--tag <value>` or `facet_set` payload violates the tag grammar (§2.3.5) | Message names the offending character / length / reserved-literal violation; suggests a sanitized form. |

Cross-clone divergence is **not** surfaced through dedicated error codes in 2.0 — it appears
as ordinary Git push/fetch failures (§8.2). Topic-related codes
(`TopicNotFound` / `ThreadNotInTopic` / `AttachToArchivedTopic` / `ShortIndexInPersistedRef`
/ `AmbiguousReferenceWithoutMarker`) and tag-vocabulary diagnostics
(`UnknownTag` / `UnknownPolicyTag` / `TagDeprecated`) are removed along with the features
they reported on.

## 14. Testing strategy

Unchanged from SPEC.md §14, plus:

### Migration

- Every state in every 1.x kind round-trips to a defined 2.0 state.
- Existing thread-to-thread links are preserved unchanged.

### Facet model

- Facet expression evaluator tests covering all guard scoping forms (`lifecycle=...`,
  `tag=...`, `AND`/`OR`/`NOT`).
- Kind preset commands (`new rfc/dec/task/bug`) produce identical facet/tag combinations as the
  canonical `thread new --lifecycle ... --tag ...` form.

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
  Migration MUST emit a warning naming any `policy.toml` line that still references it
  (per ADR-006 Consequences).

### Type-marker omission (§6.1.1)

- `git forum thread show a3f9b2k1` resolves identically to `git forum thread show @a3f9b2k1`.
- The `@` marker remains the canonical display form in CLI output.

### Linked-thread advisory display (§9.3)

- `thread show --tree` lists threads linked from the named thread (depth 1 by default) with
  current state. The tree display does not block any operation.
- `verify` may surface advisories about linked threads' states; the verification result
  itself is computed only from the named thread.

## 15. Non-goals

In addition to SPEC.md §15 and the five non-goals in `doc/spec/CORE-VALUE.md`:

- General-purpose project management (Gantt, dependency graphs).
- A topic / handle / alias / attach-detach mechanism. Earlier 2.0 drafts introduced a
  topic entity; it has been removed in favor of existing thread-link relations
  (§2.1, CORE-VALUE.md litmus). The `!` markup symbol, `topic_*` event types, the
  `refs/forum/topics/` and `refs/forum/aliases/` ref trees, and the `/N` topic-scoped
  short-index do not exist in 2.0.
- User-defined required facet axes beyond `lifecycle` (use `tags` instead).
- A `git forum push` / `git forum fetch` command or cross-clone conflict-resolution
  protocol. Distribution is plain Git on `refs/forum/*` (§8.2, CORE-VALUE.md non-goal §3).
- A tag registry, conventional-tag list, unknown-tag warnings, deprecation surfacing, or
  tag-vocabulary policy lint. Earlier drafts of 2.0 specified `.forum/tags.toml` and
  related diagnostics; these are removed in 2.0 and deferred per §2.3.5.

## Appendix A: Open questions

### A.1 Resolved during 2.0 drafting

| ID | Question | Resolution |
|---|---|---|
| O-2 | Are 5 intent values enough? | **Dropped entirely**, and `scope` was dropped too. Sole required facet is `lifecycle`; everything else (bug/task/cross-cutting) is a tag. (§2.3) |
| O-4 | Should free-form tags have any constraint, given the language-drift risk (`bug` vs `defect` vs `issue`)? | **Grammar only.** Hard tag grammar (`[a-z][a-z0-9-]{1,31}`); no registry, no conventional-tag list, no unknown-tag diagnostic, no policy lint over tag vocabulary. Drift remediation is deferred per F-T1 (Appendix A.3) until dogfood evidence shows the grammar is insufficient. (§2.3.5) |
| O-5 | Should the ten 1.x node types be preserved, or reduced? | **Reduced to four** by protocol effect: `comment`, `approval`, `objection`, `action`. The standalone Approval concept folds into the `approval` node. See ADR-006 / §2.5. |
| O-6 | Should 2.0 ship a `git forum push` / `git forum fetch` and cross-clone conflict-resolution protocol? | **No.** Distribution is delegated to plain Git on `refs/forum/*`. CORE-VALUE.md non-goal §3 forbids reinventing the protocol. (§8.2) |
| O-7 | Should 2.0 introduce a topic entity (named groupings of related threads)? | **No.** Earlier drafts introduced topic + handle + alias + attach. The grouping need is empirically "an RFC plus its `--rel implements` children", which existing thread-link relations already cover. The visualization need is met by an advisory `thread show --tree` (§9.1), not by a new entity. (§2.1) |

### A.2 Remaining for 2.0 implementation

(none currently outstanding — to be added as implementation surfaces design questions)

### A.3 Deferred from Level XS scoping (forward-compatibility plan)

The following capabilities were considered for 2.0 and **deliberately deferred** to keep the
release scope tight. Each can be added in a 2.x minor release without breaking 2.0 clients,
provided the additive contracts below are honored.

| ID | Capability | Current 2.0 substitute | Trigger to add | Forward-compat contract |
|---|---|---|---|---|
| F-T1 | Tag-vocabulary discipline (registry, conventional list, deprecation, lint) | None — bare grammar only (§2.3.5) | Documented language drift across clones (`bug` vs `defect`) producing search/policy split | Re-introduce `.forum/tags.toml` with the schema described in earlier 2.0 drafts (`description`, `aliases`, `deprecated`, `replaced_by`). All write paths emit warnings only by default; strict mode is opt-in. |
| F-A1 | Cascade state changes across linked children (`git forum deprecate <RFC> --cascade`) | None — user changes states one at a time | Repeated user requests to retire whole RFC + implementing-task groups in one step | Implement as a CLI batch (an explicit user-initiated walk of the link tree applying the same state change). Per CORE-VALUE non-goal §1, this MUST remain user-initiated batch execution; it MUST NOT become an automatic policy trigger fired by another thread's state. |

Earlier draft entries for topic-related forward-compat (F-W1 topic state machine, F-W2
topic guards, F-W3 derived health, F-W4 topic nesting, F-W5 HLC, F-W6 CRDT tags) have
been removed along with the topic entity. They are not deferred; they are out of scope.

#### Trigger discipline

A future minor release SHOULD add a deferred capability only when:

1. Documented dogfood evidence shows the substitute is insufficient.
2. The additive contract above is honored (no breaking change for clients on prior minor).
3. The corresponding ADR is written and accepted.

Speculative implementation of F-T1 / F-A1 without these triggers is explicitly discouraged.

## Appendix B: Examples

These examples illustrate the 2.0 model end-to-end. Output formatting is illustrative; exact
column layouts may differ.

### B.1 Quick bug capture

```text
$ git forum new bug "TUI crashes on terminal resize"
created thread @a3f9b2k1
  lifecycle:  execution
  tags:       bug
  status:     open

$ git forum comment @a3f9 "Resize handler doesn't account for negative width on shrink"
appended comment node n-5h2m9p1k

$ git forum evidence add @a3f9 --kind file --ref src/tui/render.rs:42
appended evidence n-7c4d8e3a

# After fix lands:
$ git forum close @a3f9 --comment "Fixed in commit 7c8d2e1"
state: open -> done
```

### B.2 RFC with implementing tasks (link-based group)

```text
$ git forum new rfc "Replace synchronous gateway with async queue" --edit
created thread @x9k2m4p7
  lifecycle:  proposal
  tags:       cross-cutting
  status:     draft

$ git forum comment   @x9k2 "Q: How do we handle ordering invariants in the queue?"
$ git forum objection @x9k2 "Async retries can violate at-most-once delivery"

# After review:
$ git forum resolve @x9k2 n-9b3c4d5e
$ git forum comment @x9k2 "Decision: queue-based dispatch with idempotency keys"
$ git forum propose @x9k2          # draft -> open
$ git forum state   @x9k2 review   # open -> review
$ git forum accept  @x9k2 --approve human/alice
state: review -> done

$ git forum new task "Implement async queue dispatcher" \
    --link-to @x9k2 --rel implements \
    --branch feat/async-dispatcher
created thread @y3p7n2q4
  lifecycle:  execution
  tags:       task
  status:     open
  links:      implements @x9k2m4p7
  branch:     feat/async-dispatcher

$ git forum show @x9k2 --tree
@x9k2m4p7  proposal/done    Replace synchronous gateway with async queue
└── @y3p7n2q4  execution/open   Implement async queue dispatcher
```

`thread show --tree` is an advisory display (§9.1, §9.3): it walks the link relations and
shows linked children with their states, but it does not gate any operation. This replaces
the topic-as-grouping affordance from earlier 2.0 drafts.

### B.3 Lightweight decision record

```text
$ git forum new dec "Use UUIDv7 for new entity IDs" --edit
created thread @q8w2e1r3
  lifecycle:  record
  tags:       (none)
  status:     open

$ git forum close @q8w2
state: open -> done
```

`lifecycle=record` skips the `working` / `review` states — records go straight to `done`.

### B.4 Listing

```text
$ git forum ls --status open
ID         TITLE                                       LIFECYCLE  TAGS    UPDATED
@a3f9b2   TUI crashes on terminal resize              execution  bug     2026-04-28
@y3p7n2   Implement async queue dispatcher            execution  task    2026-04-28
@7m4k9p   How does retry policy interact with quotas? execution  bug     2026-04-26

$ git forum thread ls --lifecycle execution --tag bug --status open
ID         TITLE                                  CREATED
@a3f9b2   TUI crashes on terminal resize         2026-04-25
@7m4k9p   How does retry policy interact ...     2026-04-26
```

### B.5 Tag-driven policy customization

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
- ADR-004 — Migration strategy.
- ADR-006 — Node type reduction (collapses 10 types to 4 by protocol effect).
- (ADR-003 — topic handle scheme — was removed when topic was dropped in favor of
  link-based grouping; see §2.1 and CORE-VALUE.md litmus.)
- (ADR-005 — cross-clone conflict resolution — was removed when distribution was
  delegated to plain Git; see §8.2 and CORE-VALUE.md non-goal §3.)
- RFC-0027 — Topic meta-thread (rejected by this draft; the cross-thread workflow
  enforcement that motivated RFC-0027 is a CORE-VALUE non-goal, and the grouping
  affordance is met by existing link relations rather than a topic entity).
- RFC-0030 — Thread ID scheme (extended: kind-named prefixes drop entirely; the `@` type
  marker becomes the display form per §6.1 and §6.2; storage is the bare 8-char token).
- RFC-0031 — 3-letter kind prefixes (deprecated by this draft).
