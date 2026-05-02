# ADR-002: Thread Kind Reduction

## Context

git-forum 1.x defines four thread kinds (`rfc`, `dec`, `task`, `issue`), each with its own state
machine, command surface, policy keys, ID prefix, and templates. The taxonomy was borrowed from
prevailing tools (IETF RFC, ADR, Linear/Jira tickets, GitHub issues) rather than derived from a
first-principles analysis of software-development activity.

Dogfood evidence in this repository (~250 threads) shows:

- **`dec` was used 0 times.** The "lightweight design decision" use case did not surface in
  practice — decisions were either captured as `summary` nodes inside other threads or recorded
  in `rfc`s.
- **`task` saw 10 uses** after introduction (RFC-0021), all small. The boundary against `issue`
  was inconsistent.
- **`issue` (`ASK`) absorbed 197 of 250 threads**, indistinguishably mixing bug reports, small
  feature requests, and ad-hoc questions.
- **`rfc` was the only kind with a coherent, stable identity** — but identity came from the
  topic shape (proposal → review → accept), not from the kind label.

Maintenance cost of four parallel state machines, four sets of operation checks, and four
ID-prefixed command groups is non-trivial — each new feature must be replicated four times.

## Decision

Replace the four-kind taxonomy with a single `thread` entity carrying:

- One **required facet**: `lifecycle ∈ {proposal, execution, record}` — gates the state machine.
- Optional **`tags[]`** — first-class, queryable, policy-referenceable. Used for sub-categories
  like `bug`, `task`, `cross-cutting`.

Lifecycle filters the unified state machine (one set of states with a per-lifecycle subset of
allowed transitions; spec §3.2). Policy uses lifecycle + tag predicates instead of kind-scoped
keys.

The four 1.x kinds map to canonical (lifecycle, tag) pairs. The kind-named top-level commands
remain in the CLI as the **stable, first-class everyday surface** — not as compatibility
shorthand on a deprecation timeline:

| 1.x kind | lifecycle | conventional tag |
|---|---|---|
| `rfc` | `proposal` | `cross-cutting` |
| `dec` | `record` | (none) |
| `task` | `execution` | `task` |
| `issue` | `execution` | `bug` |

`git forum new rfc/dec/task/bug` and the corresponding state-change shorthands
(`accept` / `close` / `pend` / etc.) remain supported across all 2.x and 3.x releases. The
`thread new --lifecycle ...` canonical form is the power-user / scripting interface; everyday
capture stays at the kind-named surface to keep friction near zero. Only kind-prefixed
*subcommand* groupings (`git forum rfc new`, `git forum issue close`) are deprecated for
removal in 3.0 (see ADR-004).

Thread IDs lose the kind prefix: 1.x `RFC-XXXXXXXX` becomes 2.0 `@XXXXXXXX` (the `@` is the
type marker; storage uses the bare 8-char token). Legacy IDs continue
to resolve via the alias mechanism.

### Example

The user types familiar nouns; the system translates each to the underlying
`(lifecycle, tag)` pair without exposing the schema:

```text
$ git forum new bug "TUI crashes on resize"
created @a3f9b2k1  (lifecycle: execution, tags: bug, status: open)

$ git forum new rfc  "Replace gateway with async queue"
created @x9k2m4p7  (lifecycle: proposal,  tags: cross-cutting, status: draft)

$ git forum new task "Implement async dispatcher" --link-to @x9k2 --rel implements
created @y3p7n2q4  (lifecycle: execution, tags: task, status: open)

$ git forum new dec  "Use UUIDv7 for new entity IDs" --link-to @x9k2 --rel relates-to
created @q8w2e1r3  (lifecycle: record,    tags: -, status: open)
```

The same flow works using the canonical form for scripts that want explicit control:

```text
$ git forum thread new "Replace gateway with async queue" \
    --lifecycle proposal --tag cross-cutting
```

Grouping across these threads is expressed via `--rel implements` link relations and surfaced
through advisory display (`show --tree`, `verify` linked-thread advisory), per CORE-VALUE.md.
2.0 deliberately rejects a topic entity — see CORE-VALUE.md §"Rejected from scope".

## Consequences

- One state machine instead of four. New states / transitions are added in one place.
- Operation-check policy keyed by lifecycle + tag is more expressive than kind-keyed (per-tag
  rules, predicate-based guards).
- Thread IDs no longer self-describe their kind. Tooling must show the lifecycle/tags in `show`
  and `ls` output.
- Kind presets are the **stable everyday surface**, not deprecated shorthand. Users — and
  agents — can keep typing `new bug` / `new task` / `new rfc` / `new dec` indefinitely. The
  facet vocabulary is internal schema, surfaced explicitly only when the user opts into
  `thread new --lifecycle ...` or writes facet-scoped policy.
- `dec` users (none observed) lose a dedicated kind but gain `lifecycle=record` threads with no
  required tag — strictly more flexible.
- Migration must inject `facet_set` events into existing thread histories (see ADR-004).
- Adding a new sub-category in the future is a tag, not a code change. Adding a new lifecycle is
  a state-machine change — deliberately higher friction.

## Alternatives

### Keep four kinds; just deprecate `dec`

Pros:

- minimal change, preserves muscle memory

Cons:

- doesn't address the maintenance cost of parallel state machines
- doesn't fix the `issue`/`task` boundary problem
- still encodes kind in IDs, blocking future taxonomy evolution

### 3-axis facet model (`intent` × `lifecycle` × `scope`)

Drafted in early SPEC-2.0 revisions. Pros: maximum expressivity. Cons: dogfood evidence didn't
support `intent`'s 5 values (decision: 0 demand, question: node-level, observation/work/claim:
body framing). `scope` carried only 1 bit of information meaningful only for proposals. Both axes
collapsed to tags.

### 2-axis (`lifecycle` × `scope`)

Intermediate option. Pros: keeps coarse cross-cutting flag. Cons: `scope` only varies for
`lifecycle=proposal`; two facets where one suffices. Replaced by `tag=cross-cutting` convention.

### No required facet (pure tags)

Pros: maximum simplicity. Cons: state machine has no anchor — it would need to dispatch on tags,
making lifecycle-set membership a soft convention rather than a hard contract. Verification and
guards become harder to reason about.

## Exit criteria

- Spec §2.3 defines `lifecycle` as the sole required facet.
- Spec §3.2 defines the unified state machine and per-lifecycle allowed states.
- Spec §7 defines facet-scoped guards and tag-keyed operation checks.
- Migration tool (ADR-004) writes `facet_set` events for all existing threads.
- Kind preset commands (`git forum new rfc`, `new task`, `new bug`, `new dec`) are implemented
  as first-class CLI surface that internally delegates to the canonical
  `thread new --lifecycle ...` path.
- All four 1.x state machines round-trip into the unified model in tests.
