# ADR-006: Node Type Reduction

## Context

git-forum 1.x defines ten node types (SPEC.md §4.3): `claim`, `question`,
`objection`, `evidence`, `summary`, `action`, `risk`, `review`,
`alternative`, `assumption`. The set was assembled top-down from the
"structured discussion" idea — each type names a rhetorical move a
participant might make.

Two issues with this taxonomy surfaced in dogfood and in the SPEC-2.0
core-value review:

1. **Two unrelated axes are conflated.** Some node types describe a
   *rhetorical move* the speaker is making (`claim`, `question`,
   `summary`, `risk`, `review`, `alternative`, `assumption`). Others
   describe a *protocol effect* the system must observe (`objection`
   blocks state transitions; `action` creates a tracked obligation;
   `approval` — currently a separate top-level concept in SPEC.md §2.7
   — gates state transitions). Mixing the two on the same enum makes
   it hard to reason about which types matter to the state machine
   versus which are body-prose framing.

2. **Empirical usage is concentrated.** In the project's own forum
   (~250 threads), nodes typed `claim` / `question` / `summary` /
   `review` / `risk` are essentially substitutable: the rhetorical
   distinction lives in the body, and policy / state machine logic
   never branches on which one was used. Node types `alternative` and
   `assumption` are rarely used at all. The genuinely structural
   types are `objection` and `action`.

3. **CORE-VALUE constraint.** `doc/spec/CORE-VALUE.md` states the
   tool's job is to keep human-and-agent discussion adjacent to code
   without orchestration. Rhetorical typing of comments does not
   serve that goal — body prose conveys the rhetoric — but
   protocol-effect typing (does this block? does this create an
   obligation? does this approve?) does, because agents and humans
   need to know what the system will do next.

## Decision

Reduce the node type set to **four**, cut by *protocol effect*:

| Node type | Protocol effect | Replaces in 1.x |
|---|---|---|
| `comment` | None — body-prose contribution | `claim`, `question`, `summary`, `risk`, `review`, `alternative`, `assumption` |
| `approval` | Positive — counts toward state-transition guards (e.g. `one_human_approval`) | the standalone Approval concept in SPEC.md §2.7 |
| `objection` | Negative — blocks state transitions until `resolve`d | `objection` (unchanged) |
| `action` | Obligation — creates a tracked work item that must be `resolve`d before terminal states | `action` (unchanged) |

`evidence` remains a **first-class non-node concept**, attached via
`evidence add` (unchanged). It was always categorically different from
the rhetorical types — it points at code, not at prose — and the 1.x
treatment of it as a node type is preserved as a CLI surface but no
longer counted in the node taxonomy of this spec.

The standalone Approval concept (SPEC.md §2.7) is folded into the node
namespace as `approval`. Existing approval semantics — gating state
transitions, the `--approve <actor>` flag on state-change commands —
continue to work; the `--approve` flag becomes a shortcut for
"append an `approval` node and apply the state change in one
event". Storage layout becomes uniform (everything is a node event)
and policy guards key off node type rather than a parallel
approval table.

CLI shorthand commands likewise reduce:

| 1.x shorthand | 2.0 |
|---|---|
| `claim`, `question`, `summary`, `risk`, `review` | `comment` |
| `objection` | `objection` (unchanged) |
| `action` | `action` (unchanged) |
| `approve` | `approve` (unchanged; emits `approval` node) |
| `retype` | retained but with the reduced type set |
| `retract`, `resolve`, `reopen` | unchanged |

## Migration

`git forum migrate` (SPEC-2.0 §10) rewrites historical nodes:

| 1.x type | 2.0 type | Notes |
|---|---|---|
| `claim`, `question`, `summary`, `risk`, `review`, `alternative`, `assumption` | `comment` | The 1.x type label is preserved as a `legacy_subtype` field on the migrated event for archival reference; queries against it are not part of the supported surface. |
| `objection` | `objection` | unchanged |
| `action` | `action` | unchanged |
| `evidence` | retained as evidence (not migrated to a node type) | unchanged surface |

Existing approvals (events of the standalone Approval kind) are
rewritten as `approval` node events.

## Consequences

- Three parallel state machines for "rhetorical types" disappear; the
  state machine and policy layer key only on the four protocol-effect
  categories.
- The policy predicate `at_least_one_summary` is **removed**. No
  replacement is shipped: in practice, a project that needs a forced
  summary before close can require a non-empty body section via
  `creation_rules.<lifecycle>.body_sections`. ADR-005's
  `at_least_one_action`-style predicates remain unchanged.
- TUI node-detail rendering simplifies (4 colors / icons instead of
  10).
- Search loses the ability to filter by `type=summary`, `type=risk`,
  etc. Body search continues to work for the rhetorical distinction
  the user actually wants. Power-users can grep the body for `Risk:`
  / `Summary:` conventions if they want a soft typology, but the
  spec does not enforce one.
- Authors who were using `claim` vs `question` to convey rhetorical
  intent now write that intent in the body (e.g. starting with
  "Claim: ..." or "Q: ..."). This is the existing behavior for many
  contributors already.

## Alternatives

### Keep all 10 types

Pros: no migration; preserves rhetorical specificity.

Cons: dogfood shows 5 of the 10 are rarely used; rhetorical typing
is body-conveyed in practice; policy / state machine never branches on
the rhetorical types, so the typing earns no enforcement value.

### Reduce to 5 (claim / question / objection / summary / action)

Pros: smaller cut, preserves the most common rhetorical types.

Cons: Mixes rhetorical types (`claim`, `question`, `summary`) with
protocol-effect types (`objection`, `action`) on the same enum —
the original problem. Decisions about whether the system does
anything with a node still require a case-split.

### Reduce to 1 (`comment` only)

Pros: maximum simplicity.

Cons: Loses the protocol distinction. `objection` blocking transitions
is load-bearing; `action` as a tracked obligation is load-bearing;
`approval` gating transitions is load-bearing. Folding these into
`comment` means the system cannot tell from the node which behavior
applies, and the body of a comment cannot drive state-machine
behavior. Rejected.

## Exit criteria

- SPEC-2.0 §2.5 overrides SPEC.md §4.3 with the four-type list and
  references this ADR.
- `git forum migrate` rewrites 1.x node events per the table above.
- CLI shorthand commands match the table above; deprecated
  shorthands print a redirect for one minor release before removal in
  3.0.
