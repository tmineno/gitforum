# ADR-004: Migration Strategy from 1.x to 2.0

## Context

git-forum 2.0 makes structural changes that cannot be expressed as soft additions to 1.x:

- Thread `kind` field is removed; replaced by `facets.lifecycle` and `tags` (ADR-002).
- Thread ID prefix changes from per-kind (`RFC-`, `ASK-`, etc.) to unified type-marker (`@` for
display; bare token for storage).
- Four state machines collapse to one (lifecycle-filtered).
- Ten node types collapse to four (`comment` / `approval` / `objection` / `action`,
  ADR-006).
- Policy key vocabulary changes from kind-named to lifecycle/tag-keyed.

Existing repositories using git-forum 1.x must be able to upgrade without losing data or
discussion history, and without forcing every team to coordinate the cutover.

Earlier 2.0 drafts also introduced a topic entity (ADR-003); that mechanism was removed
during CORE-VALUE alignment and is not part of the migration surface.

## Decision

Adopt a **hard break with one-shot migration plus short-term compatibility aliases**.

### One-shot migration (`git forum migrate`)

Performs in place:

1. **Rewrite thread refs**: `refs/forum/threads/RFC-0001` →
   `refs/forum/threads/<thread-id>` (storage form per spec §5.1 / §6.2; display form
   `@<thread-id>`). The old name is preserved as a read-only alias entry so external links
   (`RFC-0001`, `ASK-XXXXXXXX`, etc.) keep resolving.
2. **Append `facet_set` event** to every existing thread populating `lifecycle` and conventional
   tags per the kind mapping (ADR-002).
3. **Remap states** per spec §3.1.2 (lossless mapping table).
4. **Rewrite node events** per ADR-006 / spec §2.5: 1.x types `claim` / `question` /
   `summary` / `risk` / `review` / `alternative` / `assumption` migrate to `comment` (with
   `legacy_subtype` preserved); standalone Approval events migrate to `approval` nodes.
5. **Auto-rewrite policy keys** in `.forum/policy.toml` from kind-named (`creation_rules.rfc`)
   to lifecycle-named (`creation_rules.proposal`), warning on each rewrite. Emit a warning
   for any line referencing the now-removed `at_least_one_summary` predicate (ADR-006).

`git forum migrate --dry-run` reports the planned rewrite without modifying refs.

### What is permanent

The kind-named **top-level** commands and shorthands are **not** part of the deprecation story:

- `git forum new rfc` / `new dec` / `new task` / `new issue` / `new bug`
- State-change shorthands: `accept`, `close`, `pend`, `propose`, `reject`, `deprecate`

These are the stable everyday surface (ADR-002). Users keep typing them indefinitely; only the
underlying schema changes.

### Compatibility aliases (deprecated, scoped to 2.0)

In 2.0:

- Legacy thread IDs (`RFC-0001`, `ASK-XXXXXXXX`) resolve via the alias table on read.
- Legacy policy keys auto-rewrite at config-load time with a warning.
- Legacy search queries (`kind:rfc`) auto-translate to facet predicates.

The **kind-prefixed subcommand** groupings (`git forum rfc new`, `git forum issue close`,
etc.) are **removed in 2.0** — see Removal schedule below for the rationale for pulling
this forward from the previously-planned 3.0 removal.

### Removal schedule

Applies to the deprecated items above only. The permanent kind-named top-level commands
(`git forum new rfc/task/bug/dec`, `accept`, `close`, etc.) are not on this schedule and
remain supported indefinitely.

| Version | Kind-prefixed subcommands | Kind-keyed policy | Legacy IDs |
|---|---|---|---|
| 2.0 | **removed** | auto-rewrite + warning | resolve via alias |
| 2.x | — | warn on use | resolve via alias |
| 3.0 | — | rejected (must be migrated) | read-only resolve |

#### Why pull subcommand-group removal forward from 3.0 to 2.0

Previous drafts staged the kind-prefixed subcommand removal as silent-alias (2.0) → warn
(2.1) → remove (3.0). RFC-nm3d31yk's review of the v1.2.0 source tree showed that the
six `Ask` / `Issue` / `Rfc` / `Dec` / `Job` / `Task` clap variants in `main.rs` (which
all delegate to the same `ThreadCmd`) account for ≈ 500 LOC of dead duplication that
blocks the kind-reduction cleanup pass. Carrying them through 2.x defeats the
"core layer ≈ 1/3 reduction" target that motivates 2.0 in the first place.

Removing them in 2.0 directly is acceptable because:

- Top-level kind-named commands (`new rfc`, `close`, `accept`, etc.) — the muscle-memory
  surface — are unchanged and remain permanent.
- The removed forms were already documented as deprecated since 1.x (see SPEC.md §9.2 /
  §9.3 / §9.6 "remain as hidden aliases for backward compatibility").
- Dogfood inspection finds the project's own scripts and docs converged on the top-level
  form years before this removal; the alias paths are already cold.

## Consequences

- A repo can upgrade to 2.0 in one command. The migration is idempotent (running it on an
  already-migrated repo is a no-op).
- After migration, every existing thread is a flat 2.0 thread with `lifecycle` + `tags`.
  The default `git forum ls` shows them in the same flat list as new threads (no
  separate "inbox" or "untriaged" section, since topics no longer exist).
- 1.x clients cannot read 2.0-migrated repos (the new ref tree shape and event types are not
  understood). This is a true breaking change requiring all collaborators to upgrade in
  coordination.
- All discussion history, evidence, and node lifecycle is preserved — only the surrounding
  taxonomy and ID surface change.
- Custom user policy that mentions kinds in user-defined predicates (rare) requires manual
  update. The compat layer covers structured keys, not free-form expressions.
- Pulling kind-prefixed subcommand removal forward to 2.0 (rather than 3.0) shortens the
  staged-deprecation window but unlocks the LOC reduction targeted by RFC-nm3d31yk.
  Scripts that used `git forum rfc new` style invocations need updating to the top-level
  form before upgrading; the migration log emits a one-time warning naming any such
  occurrences detected in shipped helper scripts under `.forum/`.

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

### Soft data migration but hard CLI break

Pros: data-preserving, no read-time compat needed.

Cons: schema migration of refs is the unavoidable hard break — once a 2.0 client touches a
1.x repo it must rewrite refs. Splitting CLI from data migration adds complexity without
reducing breakage.

## Exit criteria

- `git forum migrate` implemented; idempotent; `--dry-run` reports planned changes.
- Lossless state-mapping test: every 1.x state in every kind round-trips to a defined 2.0 state.
- Kind-prefixed subcommand groupings (`<kind> new`, `<kind> ls`, `<kind> close`) are
  **removed in 2.0**; invoking them prints a hard error pointing at the top-level form.
- Top-level kind preset commands (`new rfc/dec/task/bug`, `close`, `accept`, etc.) are
  preserved as the stable everyday surface.
- Compat ID resolver accepts `RFC-NNNN`, `RFC-XXXXXXXX`, `ASK-NNNN`, `ASK-XXXXXXXX`,
  `JOB-XXXXXXXX`, `DEC-XXXXXXXX`, and the new `@XXXXXXXX` / bare `XXXXXXXX`.
- Removal schedule documented in release notes for 2.0 (subcommand groupings removed),
  2.x (continued kind-keyed-policy warnings), and 3.0 (kind-keyed-policy hard reject).
- Migration log captures every rewritten ref, every node-type rewrite, every policy-key
  rewrite (including `at_least_one_summary` warnings), and every kind-prefixed subcommand
  occurrence detected in shipped `.forum/` scripts.
