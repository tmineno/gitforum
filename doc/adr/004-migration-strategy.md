# ADR-004: Migration Strategy from 1.x to 2.0

## Context

git-forum 2.0 makes structural changes that cannot be expressed as soft additions to 1.x:

- Thread `kind` field is removed; replaced by `facets.lifecycle` and `tags` (ADR-002).
- Thread ID prefix changes from per-kind (`RFC-`, `ASK-`, etc.) to unified type-marker (`@` for
display; bare token for storage).
- Four state machines collapse to one (lifecycle-filtered).
- New first-class `topic` entity with its own ref tree (ADR-003).
- Policy key vocabulary changes from kind-named to lifecycle/tag-keyed.

Existing repositories using git-forum 1.x must be able to upgrade without losing data or
discussion history, and without forcing every team to coordinate the cutover.

## Decision

Adopt a **hard break with one-shot migration plus short-term compatibility aliases**.

### One-shot migration (`git forum migrate`)

Performs in place:

1. **Rewrite thread refs**: `refs/forum/threads/RFC-0001` → `refs/forum/threads/<new-id>`.
   The old name is preserved as a read-only alias entry so external links keep resolving.
2. **Append `facet_set` event** to every existing thread populating `lifecycle` and conventional
   tags per the kind mapping (ADR-002).
3. **Remap states** per spec §3.2.2 (lossless mapping table).
4. **Leave threads orphan** — no synthetic `!_legacy` topic is created. Users attach
   threads to topics manually as triage proceeds. `doctor` reports the orphan count.
5. **Auto-rewrite policy keys** in `.forum/policy.toml` from kind-named (`creation_rules.rfc`)
   to lifecycle-named (`creation_rules.proposal`), warning on each rewrite.

`git forum migrate --dry-run` reports the planned rewrite without modifying refs.

### What is permanent

The kind-named **top-level** commands and shorthands are **not** part of the deprecation story:

- `git forum new rfc` / `new dec` / `new task` / `new issue` / `new bug`
- State-change shorthands: `accept`, `close`, `pend`, `propose`, `reject`, `deprecate`

These are the stable everyday surface (ADR-002). Users keep typing them indefinitely; only the
underlying schema changes.

### Compatibility aliases (deprecated)

For one minor release after 2.0 (i.e., 2.0.x and 2.1.x), the **kind-prefixed subcommand**
groupings work as silent aliases:

- `git forum rfc new`, `git forum issue close`, etc. → expand to the top-level form.
- Legacy thread IDs (`RFC-0001`, `ASK-XXXXXXXX`) resolve via the alias table on read.
- Legacy policy keys auto-rewrite at config-load time with a warning.
- Legacy search queries (`kind:rfc`) auto-translate to facet predicates.

### Removal schedule

Applies to the deprecated items above only. The permanent kind-named top-level commands are
not on this schedule.

| Version | Kind-prefixed subcommands | Kind-keyed policy | Legacy IDs |
|---|---|---|---|
| 2.0 | silent alias, `--help` cross-references the top-level form | auto-rewrite + warning | resolve via alias |
| 2.1 | warn on use | unchanged | resolve via alias |
| 3.0 | removed | rejected (must be migrated) | read-only resolve |

## Consequences

- A repo can upgrade to 2.0 in one command. The migration is idempotent (running it on an
  already-migrated repo is a no-op).
- Immediately after migration, every existing thread is **standalone** (no topic attached).
  The default `git forum ls` mixed view (spec §9.3) shows them in the inbox section, so they
  remain visible without flag rituals — `doctor` calls them "untriaged standalone", not
  "orphan", to reflect that this is a legitimate steady state and not a fault. Users curate
  threads into topics at their own pace; many threads will never need a topic.
- 1.x clients cannot read 2.0-migrated repos (the new ref tree shape and event types are not
  understood). This is a true breaking change requiring all collaborators to upgrade in
  coordination.
- All discussion history, evidence, and node lifecycle is preserved — only the surrounding
  taxonomy and ID surface change.
- Custom user policy that mentions kinds in user-defined predicates (rare) requires manual
  update. The compat layer covers structured keys, not free-form expressions.
- The schedule (silent → warn → remove) gives users two minor releases of overlap to update
  scripts and documentation.

## Alternatives

### Coexistence (1.x and 2.0 in parallel ref trees)

Pros: gradual adoption, no flag-day cutover.

Cons: doubles maintenance burden permanently. Two state machines, two policy formats, two
sets of commands. Cross-tree links would need additional design. Locks the codebase into
supporting both forever.

### Soft migration (read-only compat for 1.x format indefinitely)

Pros: never breaks old clients.

Cons: same as coexistence. The 1.x model becomes a permanent shadow taxonomy. Defeats the
goal of kind reduction.

### Re-init (declare 1.x repos unsupported, ask users to start over)

Pros: zero migration code.

Cons: unacceptable data loss. Discussion history, evidence, decisions all gone.

### Migrate to a single `!_legacy` bucket on import (rejected per O-1)

Pros: gives every legacy thread a home; no orphan state.

Cons: pollutes `topic ls` output indefinitely with a synthetic topic that users rarely
empty. Creates the false impression that legacy threads form a coherent workstream when in
fact they are heterogeneous and uncurated. Honest "orphan" state is preferable.

### Soft data migration but hard CLI break

Pros: data-preserving, no read-time compat needed.

Cons: schema migration of refs is the unavoidable hard break — once a 2.0 client touches a
1.x repo it must rewrite refs. Splitting CLI from data migration adds complexity without
reducing breakage.

## Exit criteria

- `git forum migrate` implemented; idempotent; `--dry-run` reports planned changes.
- Lossless state-mapping test: every 1.x state in every kind round-trips to a defined 2.0 state.
- Compat alias layer covers all 1.x command shapes (`<kind> new`, `<kind> ls`, `<kind> close`,
  etc.).
- Compat ID resolver accepts `RFC-NNNN`, `RFC-XXXXXXXX`, `ASK-NNNN`, `ASK-XXXXXXXX`,
  `JOB-XXXXXXXX`, `DEC-XXXXXXXX`, and the new `@XXXXXXXX` / bare `XXXXXXXX`.
- Removal schedule documented in release notes for 2.0, 2.1, and 3.0.
- Migration log captures every rewritten ref and warning for audit.
