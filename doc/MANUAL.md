# Manual

> **2.0 vocabulary.** A thread carries one required facet — `lifecycle`
> (`proposal` / `execution` / `record`) — plus free-form `tags`. The four
> 1.x kinds (`rfc`, `dec`, `task`, `issue`/`ask`/`bug`) live on as
> top-level **presets** that produce the conventional (lifecycle, tags)
> pair (SPEC-2.0 §9.1). Discussion uses four canonical node types —
> `comment`, `approval`, `objection`, `action` — chosen by protocol
> effect (SPEC-2.0 §2.5 / ADR-006). Thread states are unified across
> lifecycles: `draft`, `open`, `working`, `review`, `done`, `rejected`,
> `withdrawn`, `deprecated` (SPEC-2.0 §3.1). Thread IDs display as
> `@XXXXXXXX` and store as the bare 8-char base36 token (SPEC-2.0 §6).
>
> The kind-prefixed *subcommand groupings* (`git forum rfc new`,
> `git forum issue close`, etc.) were removed in 2.0 — invoking them
> prints a hard error pointing at the top-level form. Legacy 1.x thread
> IDs (`RFC-…`, `ASK-…`, `DEC-…`, `JOB-…`) keep resolving via the alias
> table after `git forum migrate`.

## Quick Reference

```
# create — kind preset (everyday)
git forum new <kind> "Title" [--body "..."|--edit]   Create via kind preset
                                                     (rfc/dec/task/issue/bug)
                                                     → maps to (lifecycle, tags)

# create — canonical (power-user, scriptable; SPEC-2.0 §9.1)
git forum thread new "Title" --lifecycle <L> [--tag <T>]...
                                                     Create with explicit
                                                     lifecycle + tags

# inspect
git forum ls [--lifecycle <L>] [--tag <T>] [--status <S>] [--branch <B>]
                                                   List threads
git forum ls --kind <kind>                         Filter by preset (legacy
                                                   alias for the equivalent
                                                   --lifecycle/--tag pair)
git forum show <ID>                                Show thread details
git forum show <ID> --what-next                    Show valid next actions
git forum show <ID> --compact                      Compact single-line view
git forum show <ID> --no-timeline                  Omit timeline from output
git forum show <ID> --tree                         List direct incoming `implements` children (advisory)
git forum brief <ID> [--json]                      Read-only single-thread digest (RFC-5wf2v8hv)
git forum log <ID>                                 Show event timeline for a thread
git forum log <ID> --reverse                       Show newest events first
git forum log <ID> -n <N>                          Limit to last N events
git forum search <query>                           Search threads and nodes
                                                   (kind:<name> auto-translates
                                                    to lifecycle:/tag: with a
                                                    deprecation warning)
git forum shortlog --since <DATE_OR_REV>           Threads resolved after date/tag
git forum status <ID>                              Check open items
git forum node show <NODE_ID>                      Inspect a single node

# discussion (canonical 2.0 + deprecated shorthands)
git forum node add <ID> --type <type> "body"       Add a typed node
git forum comment <ID> "body"                      node add --type comment (2.0 canonical)
git forum objection <ID> "body"                    node add --type objection
git forum action <ID> "body"                       node add --type action
git forum claim|question|summary|risk|review <ID> "body"
                                                   Deprecated aliases for
                                                   `comment` (warn + alias for
                                                   one minor; removed in 3.0)
git forum retype <ID> <NODE_ID> --type <TYPE>      Change a node's type
git forum resolve <ID> <NODE_ID>                   Resolve a node
git forum retract <ID> <NODE_ID>                   Retract a node
git forum reopen <ID> <NODE_ID>                    Reopen a node

# state (unified machine + lifecycle-aware shorthands; SPEC-2.0 §3.1 / §9.3)
git forum state <ID> <state>                       Change thread state
git forum state <ID> <state> --approve human/alice State change with approval
git forum state <ID> <state> --comment "Done"      State change with comment
git forum state bulk --to <state> [--kind <kind>]  Bulk state change
git forum close <ID>                               execution/record: -> done;
                                                   proposal: rejected (use `accept`)
git forum accept <ID> --approve human/alice        proposal/record: -> done;
                                                   execution: rejected (use `close`)
git forum propose <ID>                             proposal: draft -> open
git forum pend <ID>                                execution: -> working
git forum reject <ID>                              any lifecycle: -> rejected
git forum withdraw <ID>                            proposal: draft|open -> withdrawn
git forum deprecate <ID>                           any lifecycle: -> deprecated

# evidence & links
git forum evidence add <ID> --kind <kind> --ref <ref>  Add evidence
git forum link <FROM> <TO> --rel <rel>             Link two threads
git forum branch bind <ID> <branch>                Bind thread to branch

# body revision
git forum revise <ID> [--body "..."|--edit]        Revise thread body
git forum revise node <ID> <NODE_ID> --body "..."  Revise a node
git forum diff <ID>                                Diff between body revisions
git forum diff <ID> --rev N                        Diff revision N-1 vs N
git forum diff <ID> --rev N..M                     Diff revision N vs M

# policy & diagnostics
git forum verify <ID>                              Preflight check for next forward transition
git forum policy show                              Display loaded policy rules
git forum policy lint                              Check policy for problems
git forum policy check <ID> --transition from->to  Check guards for transition

# setup & maintenance
git forum init                                     Initialize forum in repo
git forum doctor                                   Check repository health
git forum reindex                                  Rebuild local index from Git refs
git forum migrate [--dry-run]                      Rewrite a 1.x repo to 2.0 storage
git forum hook install                             Install commit-msg hook
git forum tui                                      Open interactive TUI
git forum purge --thread <ID> --event <SHA>        Purge event content
git forum purge --actor <ACTOR_ID>                 Purge all events by actor
```

## Conventions

- thread classification — one required facet (`lifecycle`) and free-form `tags`. The four
  kind presets emit the conventional pair:

  | Preset            | `lifecycle`  | conventional tag |
  |-------------------|--------------|------------------|
  | `new rfc`         | `proposal`   | `cross-cutting`  |
  | `new dec`         | `record`     | (none)           |
  | `new task`        | `execution`  | `task`           |
  | `new issue`/`bug` | `execution`  | `bug`            |

  Presets are not on any removal schedule (SPEC-2.0 §10.2); they are the everyday surface.
  Repositories that need other (lifecycle, tag) combinations use the canonical
  `git forum thread new --lifecycle <L> --tag <T>...` form.
- thread IDs:
  - **Display form** is `@XXXXXXXX` (8 base36 chars, e.g. `@a7f3b2x1`); the `@` is shell-safe and
    purely a display marker — every CLI position also accepts the bare token (`a7f3b2x1`).
  - **Storage form** is the bare token under `refs/forum/threads/`.
  - Unambiguous prefixes (≥4 chars after `@`) are accepted (e.g. `@a7f3`).
  - Legacy 1.x IDs (`RFC-XXXXXXXX`, `ASK-NNNN`, `DEC-…`, `JOB-…`) keep resolving via the alias
    table after `git forum migrate` (SPEC-2.0 §10.1 / §6.2).
- node IDs: printed by node commands (e.g. `comment`, `objection`); canonical IDs are Git
  commit OIDs of the say event.
- CLI/TUI displays of node and event OIDs usually show the first 16 characters.
- node IDs in CLI arguments:
  - full IDs always work
  - if there is no exact match, a unique prefix of at least 8 characters is accepted
  - `git forum node show` resolves prefixes globally
  - `revise node`, `retract`, `resolve`, and `reopen` resolve prefixes inside the specified thread
- actor:
  - resolution order: `--as` flag → `GIT_FORUM_ACTOR` env var → Git config `user.name`
  - `--as human/alice` or `--as ai/reviewer` overrides everything
  - `GIT_FORUM_ACTOR=ai/reviewer` persists across commands without repeating `--as`
  - if neither is set, the actor is inferred from Git config as `human/<slug>`
  - actor IDs are trust-based claims for attribution; MVP does not authenticate `--as`
- commit identity (separate from actor):
  - controls the Git commit author/committer metadata on forum commits
  - defaults to Git config `user.name` / `user.email`
  - override via `[commit_identity]` in `.git/forum/local.toml`

### Trust model

git-forum uses a **trust-based** identity model:

- **Actor IDs are claimed, not authenticated.** Anyone can pass `--as human/alice`. There is no login, token, or key verification in the current version.
- **Approvals are recorded, not cryptographically verified.** An approval event stores the supplied actor ID, but nothing proves that actor actually signed off.
- **History rewriting is intentional** for some operations (`purge`). Event logs are not tamper-evident.

These trade-offs keep the tool lightweight and Git-native. Cryptographic signing of events and approvals is planned as a future extension. Until then, git-forum is best suited for teams where repository access already implies a baseline of trust.

## Picking a kind preset

The four presets are everyday shortcuts onto (lifecycle, tags). Pick by what kind of progression
the work goes through:

| If the work...                                     | Use     | Underlying facets               |
|----------------------------------------------------|---------|---------------------------------|
| Affects multiple teams, hard to reverse            | `rfc`   | `proposal` + `cross-cutting`    |
| Is a local design decision worth recording         | `dec`   | `record`                        |
| Is an implementable unit of work with clear scope  | `task`  | `execution` + `task`            |
| Is a bug report or a small request                 | `bug` / `issue` | `execution` + `bug`     |

Rules of thumb:

- If you are comparing alternatives → `dec`.
- If you are defining acceptance criteria → `task`.
- If you need cross-team sign-off → `rfc`.
- If something is broken or missing → `bug`.
- When in doubt between `dec` and `rfc`, start with `dec` — it can be elevated to an RFC later
  with `--from-thread`.
- When in doubt between `task` and `bug`, prefer `task` if you know the implementation path.

For arbitrary (lifecycle, tag) combinations beyond the four presets — e.g. an
`execution` thread tagged neither `task` nor `bug`, or a `proposal` that is not
`cross-cutting` — use the canonical form:

```bash
git forum thread new "Title" --lifecycle execution --tag spike
git forum thread new "Title" --lifecycle proposal                    # no tag
```

Agents are participants, not a separate control plane. Use the same presets and the same
canonical commands.

## Create threads

### RFC (`lifecycle=proposal`, `tag=cross-cutting`)

Use RFCs to frame work before implementation starts. RFCs follow the proposal lifecycle:
`draft → open → review → done` (or `rejected` / `withdrawn`).

```bash
git forum new rfc "Switch solver backend to trait objects"
git forum new rfc "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum new rfc "Switch solver backend to trait objects" --body -
git forum new rfc "Switch solver backend to trait objects" --body-file ./tmp/rfc.md
git forum new rfc "Switch solver backend to trait objects" --edit
```

### DEC (`lifecycle=record`)

Use DECs to record local design decisions worth preserving. Records have a short lifecycle —
`open → done` (or `rejected` / `deprecated`); they skip `working` and `review` entirely.
Default policy requires a body with `Context` / `Decision` / `Rationale` / `Impact`.

```bash
git forum new dec "Use Redis over Memcached" --body-file ./tmp/dec.md
git forum comment @<dec-id> "Alternative: Memcached — simpler, but no pub/sub."
git forum comment @<dec-id> "Assumption: Redis cluster available in prod."
git forum close @<dec-id>                # open -> done
```

If you previously used `node add --type alternative` / `--type assumption` for DECs, those
types are aliased to `comment` for one minor release with a deprecation warning (ADR-006);
record the same content in a `comment` body, optionally prefixed `Alternative:` / `Assumption:`.

### Task (`lifecycle=execution`, `tag=task`)

Use tasks for implementable work units with clear scope. Default policy requires a body with
`Background` / `Acceptance criteria` sections.

> **Alias:** `job` still works as an alias for `task` in all commands.

```bash
git forum new task "Implement Redis cache client wrapper"
git forum pend @<task-id>                         # open -> working
git forum state @<task-id> review                 # working -> review
git forum close @<task-id>                        # review -> done

# Fast-track for a trivial change that doesn't need a working/review pass:
git forum new task "Add cache-control headers"
git forum close @<task-id>                        # open -> done (allowed by execution lifecycle)
```

Execution lifecycle states: `open → working → review → done` (or `rejected` / `deprecated`).
Permissive edges (`open → done`, `working → done`) exist for trivial closes; the project's
policy decides which path is required for which tag (SPEC-2.0 §3.1.1).

### Bug / issue (`lifecycle=execution`, `tag=bug`)

Use bugs for short observation-style execution threads — bug reports, small requests, anything
that doesn't need a structured body up front.

> **Aliases:** `issue` and `ask` still work as aliases for `bug` in all commands.

```bash
git forum new bug "Implement trait backend"
git forum new bug "Implement trait backend" --body "Initial implementation checklist"
git forum new bug "Implement trait backend" --body -
git forum new bug "Implement trait backend" --body-file ./tmp/issue.md
git forum new bug "Implement trait backend" --edit
git forum new bug "Implement trait backend" --branch feat/trait-backend
git forum new bug "Implement trait backend" \
  --link-to @<rfc-id> --rel implements
git forum new rfc "Error handling" \
  --comment "All errors should be typed" --action "Define error enum"
```

`--body -` reads the initial body from standard input. `--edit` opens
`$VISUAL` / `$EDITOR` / `vi` for interactive body composition. Lines starting with `#` are
stripped. Empty content aborts the command. `--edit` conflicts with `--body` / `--body-file`
and requires an interactive terminal; in scripts or agent workflows, use `--body`,
`--body-file`, or `--body -`.

### Inline nodes at creation

Thread creation accepts inline node flags that are appended immediately after the thread is
created. `--comment`, `--objection`, and `--action` create canonical 2.0 node types. The 1.x
flags `--claim`, `--question`, `--summary`, `--risk` continue to work for one minor release
with a deprecation warning; under the hood each writes a `comment` node with `legacy_subtype`
preserved (ADR-006). Each flag may be repeated:

```bash
git forum new rfc "Caching layer" --body "Goal and constraints." \
  --comment "LRU eviction with 10-min TTL" \
  --action  "Benchmark cache hit ratio" \
  --comment "Memory pressure may be a concern under load"
```

This is equivalent to running `new rfc` followed by separate `comment` and `action` commands.
`--branch <BRANCH>` binds the new thread to an existing Git branch.
`--link-to <THREAD_ID> --rel <REL>` creates the thread and immediately records one or more links
from the new thread to existing threads.

### Create from a commit

```bash
git forum new bug --from-commit HEAD
git forum new bug --from-commit abc123 --link-to @<rfc-id> --rel implements
```

`--from-commit <REV>` uses the commit subject as the title, the commit body as the thread body,
and automatically adds the commit as evidence. An explicit title argument overrides the subject.

### Create from another thread

```bash
git forum new rfc --from-thread @<source-rfc-id>
git forum new bug --from-thread @<source-bug-id>
git forum new rfc --from-thread @<source-bug-id> "Custom title"
```

`--from-thread <THREAD_ID>` copies the title (prefixed with `v2: `) and body from the source
thread and creates bidirectional `supersedes` / `superseded-by` links. An explicit title argument
overrides the default title.

Behavior depends on the source and target presets:

- **RFC → new RFC**: source RFC is auto-deprecated (supersession).
- **Bug → new bug**: source bug is unchanged (respin/split). Prints a hint to close the source.
- **Bug → new RFC**: source bug is unchanged (elevation to formal proposal). Prints a hint to close the source.
- **RFC → new bug/task**: not allowed — use `git forum link --rel implements` instead.

### Filter the thread list

```bash
git forum ls --lifecycle proposal
git forum ls --lifecycle execution --tag bug
git forum ls --tag cross-cutting --branch feat/trait-backend
git forum ls --kind rfc                            # legacy alias; auto-translates
git forum ls dec                                   # positional preset shorthand
```

The kind-prefixed *subcommand groupings* (`git forum issue ls`, `git forum rfc ls`,
etc.) were **removed** in 2.0 (SPEC-2.0 §10.2). Invoking them prints a hard
error pointing at the top-level form (`git forum ls --lifecycle execution --tag bug`).
`--kind` continues to resolve via the preset table for the four conventional combinations.

The `ls` output column header is `LIFECYCLE`. Tags are rendered as a leading bracket prefix on
the title cell when non-empty, e.g. `[bug] search is slow`.

## Structured discussion

### Add a node

Discussion is recorded as four canonical typed nodes — `comment`, `approval`, `objection`,
`action` — chosen by **protocol effect** (SPEC-2.0 §2.5):

| Node type    | Protocol effect |
|--------------|-----------------|
| `comment`    | None — body-prose contribution. Carries questions, summaries, observations, risks, reviews, and decision rationale in body text. |
| `approval`   | Positive — counts toward state-transition guards (e.g. `one_human_approval`). Typically appended via the `--approve <ACTOR>` flag on a state-change command. |
| `objection`  | Negative — blocks state transitions until `resolve`d. |
| `action`     | Obligation — creates a tracked work item that must be `resolve`d before terminal states. |

`evidence` is a separate first-class concept attached via `evidence add`; it is intentionally
outside the node taxonomy.

Each canonical node type has a dedicated shorthand command. All node commands accept a
positional body argument, `--body-file`, `--edit`, and `--as`. Pass `"-"` as the positional body
to read from stdin. Use `--edit` to compose in `$EDITOR`.

```bash
git forum comment   @<rfc-id> "Need a stable plugin-facing boundary."
git forum comment   @<rfc-id> "Q: what compatibility risks remain?"
git forum objection @<rfc-id> "Benchmarks are missing."
git forum action    @<task-id> "Add branch-local benchmark fixture."
git forum comment   @<rfc-id> "Direction is sound; migration evidence is missing."
git forum objection @<rfc-id> --body-file ./tmp/detailed-objection.md
git forum comment   @<rfc-id> --body -
```

The 1.x rhetorical types — `claim`, `question`, `summary`, `risk`, `review`, `alternative`,
`assumption` — were collapsed into `comment` (ADR-006). For one minor release the old shorthand
commands and `node add --type <legacy>` continue to work and emit a deprecation warning; under
the hood they write a `comment` event with `legacy_subtype` preserved. Authors who relied on
the rhetorical distinction express it in the body (e.g. start with `Q:`, `Decision:`,
`Risk:`, `Alternative:`, `Assumption:`).

To record an `approval`, use the `--approve <ACTOR>` flag on a state-change command (preferred)
or `git forum node add <ID> --type approval`:

```bash
git forum accept @<rfc-id> --approve human/alice          # appends approval + transitions
git forum node add @<rfc-id> --type approval --as human/alice
```

Policy guards key off `approval` nodes uniformly (e.g. `one_human_approval`).

On success, each node command prints the node ID to stdout and a next-actions hint to stderr:

```text
Added comment 6f1d2c3b4a5e67890123456789abcdef01234567
  next: review, rejected, withdrawn
  open: 1 open objection(s)
```

The hint shows valid state transitions and open items. Suppress with `2>/dev/null`.

### Retype a node

Use `retype` to change the type of an existing node:

```bash
git forum retype @<rfc-id> 6f1d2c3b --type comment
```

The old type is recorded in the event for auditability. Accepts `--as` and `--force`.

### Revise a node

```bash
git forum revise node @<rfc-id> 6f1d2c3b4a5e67890123456789abcdef01234567 \
  --body "What is the migration and rollback plan?"
git forum revise node @<rfc-id> 6f1d2c3b \
  --body "What is the migration and rollback plan?"
git forum revise node @<rfc-id> 6f1d2c3b --edit
```

Use `revise node` to update an existing node when the intent is the same but the content needs
correction (for example, a comment whose body is being clarified). The revision history is
preserved and visible in `git forum node show`.

### Retract / resolve / reopen a node

All three commands accept one or more node IDs:

```bash
git forum retract @<rfc-id> 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve @<rfc-id> 6f1d2c3b4a5e67890123456789abcdef01234567
git forum reopen  @<rfc-id> 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve @<rfc-id> 6f1d2c3b
git forum retract @<rfc-id> node1 node2 node3   # retract multiple nodes
git forum resolve @<rfc-id> node1 node2         # resolve multiple nodes
```

- `resolve` / `reopen` are mainly for `objection` and `action`.
- `retract` is a **soft-delete**: it marks the node inactive but the original body text remains
  in git history. Anyone with repo access can read retracted content via `git log`. Do not use
  retract for removing sensitive data — there is currently no hard-delete mechanism.
- when multiple node IDs are given, each node is processed independently; failures are reported
  inline on stderr and the command exits non-zero if any fail.

### Reply to a node

Use `--reply-to` to link a node as a response to an existing node:

```bash
git forum comment @<rfc-id> "Tests added, benchmark in bench/result.csv" \
  --reply-to <OBJECTION_NODE_ID>
git forum comment @<rfc-id> "Q: can you clarify X?" --reply-to <COMMENT_NODE_ID>
```

`--reply-to` is accepted on all shorthand node commands. Reply chains of arbitrary depth are
supported. `git forum show` groups reply chains into conversations for readability.

### Inspect a single node

```bash
git forum node show 6f1d2c3b4a5e67890123456789abcdef01234567
git forum node show 6f1d2c3b
```

`git forum node show <NODE_ID>` shows:

- node ID
- the thread it belongs to
- type (and `legacy_subtype` for migrated 1.x nodes)
- current state: `open`, `resolved`, `retracted`, `incorporated`
- in reply to (if this node is a reply)
- created_at
- actor
- current body
- thread links, if the parent thread is linked to other threads
- the history related to that node

If a prefix is ambiguous, the command fails and prints candidate full IDs.

## State transitions

### Status

```bash
git forum status @<rfc-id>
```

`git forum status <THREAD_ID>` shows unresolved items grouped by type: open objections and
open actions.

### The unified state machine

A single state set replaces the four 1.x machines (SPEC-2.0 §3.1):

```text
draft   -> open | withdrawn
open    -> working | review | done | rejected | withdrawn
working -> review | done | rejected
review  -> done | working | rejected
done    -> open (reopen)  | deprecated
rejected -> open          | deprecated
```

Per-lifecycle filters (SPEC-2.0 §3.1.1):

| `lifecycle`   | Allowed states                                                   | Initial | Typical path                                |
|---------------|------------------------------------------------------------------|---------|---------------------------------------------|
| `proposal`    | `draft`, `open`, `review`, `done`, `rejected`, `withdrawn`, `deprecated` | `draft`   | `draft → open → review → done`             |
| `execution`   | `open`, `working`, `review`, `done`, `rejected`, `deprecated`    | `open`    | `open → working → review → done` (task) or `open → done` (bug) |
| `record`      | `open`, `done`, `rejected`, `deprecated`                         | `open`    | `open → done`                              |

A transition whose destination is not in the lifecycle's allowed set is rejected with
`LifecycleStateMismatch`.

### Shorthand commands

State shorthands are top-level convenience aliases (verb-first). Each is lifecycle-aware
(SPEC-2.0 §9.3):

```bash
git forum close @<bug-id>                              # execution: -> done
git forum close @<bug-id> --comment "Fixed in abc123"
git forum close @<bug-id> --link-to @<rfc-id> --rel implements
git forum close @<task-id> --resolve-open-actions
git forum pend  @<task-id>                             # execution: -> working
git forum pend  @<task-id> --comment "Waiting on review"
git forum state @<bug-id>  open                        # reopen
git forum reject @<bug-id> --comment "Won't fix"
git forum propose @<rfc-id>                            # proposal: draft -> open
git forum accept  @<rfc-id> --approve human/alice      # proposal: review -> done
git forum withdraw @<rfc-id>                           # proposal: draft|open -> withdrawn
git forum deprecate @<rfc-id> --comment "Superseded by @<successor-id>"
git forum state @<rfc-id> deprecated --link-to @<successor-id> --rel relates-to
```

| Shorthand   | `lifecycle=execution` | `lifecycle=proposal`        | `lifecycle=record` |
|-------------|-----------------------|-----------------------------|--------------------|
| `close`     | → `done`              | (rejected: use `accept`)    | → `done`           |
| `accept`    | (rejected: use `close`) | → `done`                  | → `done`           |
| `propose`   | (rejected)            | → `open` (from `draft`)     | (rejected)         |
| `pend`      | → `working`           | (rejected)                  | (rejected)         |
| `reject`    | → `rejected`          | → `rejected`                | → `rejected`       |
| `withdraw`  | (rejected)            | → `withdrawn` (from `draft`/`open`) | (rejected)  |
| `deprecate` | → `deprecated`        | → `deprecated`              | → `deprecated`     |

Shorthand commands combine a state transition with optional `--comment` (attaches comment text
to the state-change event's body), `--link-to` (creates links after transitioning), and
`--approve` (records approvals as `approval` nodes).

### Generic state command

```bash
git forum state @<rfc-id> open                                 # draft -> open
git forum state @<rfc-id> review                               # open -> review
git forum state @<rfc-id> done --approve human/alice
git forum state @<task-id> done --resolve-open-actions
git forum state @<task-id> done --comment "Done" --link-to @<rfc-id> --rel implements
git forum state bulk --to done --branch v0.1.0
git forum state bulk --to done @<task-1> @<task-2> --dry-run
```

- `--approve` records an approval node on the event.
- recorded approvals are not cryptographically verified in the MVP.
- `--comment` attaches comment text to the state-change event's body (visible in the timeline).
- `--link-to` and `--rel` create thread links after the state transition.
- whether the transition succeeds depends on the unified state machine, the lifecycle filter,
  and policy guards.
- `--resolve-open-actions` is an explicit escape hatch for state changes that hit the
  `no_open_actions` guard; it resolves open `action` nodes before writing the state event.
- `state bulk` evaluates each target independently, applies successful transitions, reports
  failures inline, and exits non-zero if any target failed. The `--kind` filter selects threads
  via the preset's (lifecycle, tag) pair.
- `state bulk --dry-run` reports what would succeed or fail without writing any events.

## Search, list, show

### List threads

```bash
git forum ls
git forum ls --lifecycle execution --tag bug --branch feat/trait-backend
git forum show @<rfc-id>
git forum show @<rfc-id> --what-next
```

`git forum ls` shows `ID`, `LIFECYCLE`, `STATUS`, `BRANCH`, `CREATED`, `UPDATED`, and `TITLE`.
Tags appear as a leading bracket prefix in the title column (e.g. `[bug] …`).
`--lifecycle` and `--tag` filter on the canonical facets; `--kind <preset>` is preserved as a
legacy alias that resolves through the preset table.
`--branch <BRANCH>` filters the listing to threads currently bound to that branch.

### Show thread details

`git forum show <THREAD_ID>` shows:

- title, lifecycle, tags, status
- **next**: compact list of valid transitions with guard status (e.g. `done (blocked: no_open_objections), rejected, withdrawn`)
- **transitions**: Unicode state diagram with current state highlighted in brackets
- created_at, created_by, branch
- body
- body revisions count (if body has been revised)
- incorporated nodes (if any)
- open objections, open actions
- evidence, links
- conversations (reply chains grouped by root node)
- timeline

`git forum show <THREAD_ID> --what-next` shows valid next actions plus operation check
rules for the current state:

```text
@a7f3b2x1 (review)

valid transitions: done, rejected, working

guard check (review -> done):
  [FAIL] no_open_objections -- 1 open objection(s)

open objections: 1
open actions:    0
nodes:           6
evidence:        1
links:           0

operation checks (state: review):
  node types: (all allowed)
  body revise: not allowed in this state
  evidence:    allowed
```

Three discoverability surfaces exist:

- **`show`**: compact, thread-specific — `next:` line and state diagram
- **`show --what-next`**: detailed, thread-specific — guard checks plus operation check rules
- **`policy show`**: global — full policy as loaded from `.forum/policy.toml`

The timeline is displayed in `date node_id event_id author type body` order.

If the thread has evidence or links attached, they appear between the body and the timeline:

```text
evidence: 1
  - a1b2c3d4  benchmark  bench/result.csv

links: 1
  - @m2k9p4n8  implements
```

### Log

`git forum log <THREAD_ID>` shows the event timeline for a thread as a standalone command.

```bash
git forum log @<rfc-id>
git forum log @<rfc-id> --reverse    # newest events first
git forum log @<rfc-id> -n 5         # last 5 events
```

This is the timeline from `git forum show` as a standalone view.

### Brief

```bash
git forum brief @<rfc-id> [--json]
```

Read-only digest of one thread for AI agents and scripts that need a stable
machine-readable surface (RFC-5wf2v8hv). **Reads only the named thread's events.** Outgoing
links are reported as counts grouped by relation; incoming links come from the SQLite
reverse-link index and are likewise reported as counts grouped by relation only — `brief`
never reads a linked thread's body, title, or state.

`--json` emits a stable v1 schema with `id`, `title`, `lifecycle`, `tags`, `status`,
`created_at`, `created_by`, `branch`, `links_in`, `links_out`, `node_counts` (keyed by
`comment` / `approval` / `objection` / `action`), `open_objections`, `open_actions`,
`evidence_count`, and `latest_summary`.

### Search

```bash
git forum search migration
git forum search objection
git forum search @<rfc-id>
git forum search "kind:rfc"           # legacy; auto-translates to lifecycle:proposal AND tag:cross-cutting
```

`git forum search <QUERY>` searches the local index across:

- thread title
- thread body
- thread lifecycle, tags, and state
- thread ID
- current node body
- current node type and node ID

Results are grouped by thread. If the match came from a current node, the output includes the
matching node under the thread row.

Legacy `kind:<name>` predicates auto-translate to the equivalent `lifecycle:` / `tag:` form for
one minor release with a deprecation warning (SPEC-2.0 §12).

## Evidence and links

### Add evidence to a thread

```bash
git forum evidence add @<rfc-id> --kind benchmark --ref bench/result.csv
git forum evidence add @<task-id> --kind commit --ref HEAD~1
git forum evidence add @<task-id> --kind commit --ref abc123def456
git forum evidence add @<task-id> --kind commit --ref abc123 def456 789012
git forum evidence add @<task-id> --kind file --ref src/lib.rs
git forum evidence add @<task-id> --kind test --ref tests/backend_trait.rs
```

`--ref` accepts multiple values in a single command. Each ref creates its own evidence event.

Valid evidence kinds: `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`, `thread`, `external`.

For `--kind commit`, `--ref` may be a full SHA, short SHA, branch, tag, or other Git revision
expression. `git-forum` resolves it to a commit and stores the canonical commit OID. If the
revision does not resolve to a commit object, the command fails.

On success, the command prints the first 8 characters of the evidence ID, which is the Git commit
SHA of the `link` event:

```text
Evidence added (a1b2c3d4)
```

### Linking implementation commits

After implementing work on a branch, link the commits back to the RFC or task so that the
decision trail connects to the code:

```bash
git forum evidence add @<task-id> --kind commit --ref HEAD
git forum evidence add @<task-id> --kind test   --ref tests/cache_test.rs
git forum evidence add @<task-id> --kind thread --ref @<rfc-id>
```

`--kind commit --ref` accepts any Git revision expression (SHA, branch, tag, `HEAD~1`). The
resolved commit OID is stored canonically.

### Link two threads

```bash
git forum link @<task-id> @<rfc-id>     --rel implements
git forum link @<task-2>  @<task-1>     --rel depends-on
git forum link @<task-3>  @<task-2>     --rel blocks
git forum link @<rfc-2>   @<rfc-1>      --rel relates-to
```

On success:

```text
@m2k9p4n8 -> @a7f3b2x1 (implements)
```

`--rel` is currently free-form. Common values are `implements`, `relates-to`, `depends-on`, and
`blocks`. `git forum show <ID> --tree` walks one hop of incoming `--rel implements` references
as an advisory display (SPEC-2.0 §2.1 / §9.1).

### Bind a thread to a Git branch

```bash
git forum branch bind @<task-id> feat/parser-rewrite
git forum branch clear @<task-id>
```

This updates the thread's `scope.branch`. It is most useful for execution threads that track
implementation work on a feature branch, but the command is available for any thread.

## Body revision and diff

### Revise thread body

`body` is the default target for `revise` — the `body` keyword is optional:

```bash
git forum revise @<rfc-id> --body "Updated body text"
git forum revise @<rfc-id> --body-file ./tmp/body.md
git forum revise @<rfc-id> --body -
git forum revise @<rfc-id> --edit
git forum revise body @<rfc-id> --body "Updated body text"   # explicit, still works
```

`--incorporates` marks referenced nodes as incorporated into this revision:

```bash
git forum revise @<rfc-id> --body "Revised body" \
  --incorporates 6f1d2c3b --incorporates a1b2c3d4
```

Incorporated nodes appear as `incorporated` status in show output, distinct from `resolved` and
`retracted`. They represent content that has been folded into the current body.

### Diff body revisions

After revising a thread body, use `diff` to see what changed between revisions:

```bash
git forum diff @<rfc-id>                            # latest vs previous revision
git forum diff @<rfc-id> --rev 1                    # diff revision 0 vs 1
git forum diff @<rfc-id> --rev 0..2                 # diff revision 0 vs 2
```

Revision numbering:

- **Revision 0**: the body from the Create event (empty string if the thread was created without a body)
- **Revision 1, 2, ...**: each subsequent ReviseBody event in timeline order

Output uses unified diff format matching `git diff` conventions. Diff headers show
`a/revN/body` and `b/revM/body` labels instead of temporary file paths.

`--rev` accepts two formats:

- `--rev N` — diff between revision N-1 and N
- `--rev N..M` — diff between revision N and M

If the thread has no body revisions, an informative message is printed instead.

## Preflight and policy

```bash
git forum verify @<rfc-id>
git forum policy show
git forum policy lint
git forum policy check @<rfc-id> --transition review->done
```

- `verify`: preflight check — tests whether the thread is ready for its next forward transition (not a history audit). May surface advisories about linked threads' states; the verification result is computed only from the named thread (SPEC-2.0 §9.4).
- `policy show`: displays the loaded policy in human-readable format (guards, creation rules, operation checks, strict mode). Only shows configured sections — no synthesized defaults.
- `policy lint`: validates `.forum/policy.toml` — checks guard syntax, unknown states, invalid transitions, and warns when allow-lists miss entire lifecycles.
- `policy check`: dry-runs guard evaluation for a specific transition.

### The policy file

The policy file lives at `.forum/policy.toml`.

It is created automatically by `git forum init`, and it controls:

- **Transition guards** (`[[guards]]`): rules that must pass for a state transition. Guard
  scopes use a boolean facet expression over `lifecycle` and `tag=<value>` membership tests
  (SPEC-2.0 §7.1).
- **Operation checks** (`creation_rules.<lifecycle>[.tag.<name>]`, `node_rules`,
  `revise_rules`, `evidence_rules`, `checks`): rules that validate write operations before
  committing events.

A 2.0 policy example:

```toml
# Guards are scoped by facet expression. Unscoped guards apply to all threads
# with the matching transition.
[[guards]]
on = "lifecycle=proposal AND tag=cross-cutting : review->done"
requires = ["one_human_approval", "no_open_objections"]

[[guards]]
on = "lifecycle=execution : open->done"
requires = ["no_open_actions"]

[[guards]]
on = "lifecycle=execution AND tag=task : review->done"
requires = ["no_open_actions", "has_commit_evidence"]

[[guards]]
on = "lifecycle=record : open->done"
requires = ["no_open_objections"]

[checks]
strict = false

# Creation rules key off lifecycle, with optional `tag.<name>` specialization.
# Most-specific match wins (creation_rules.execution.tag.task overrides
# creation_rules.execution).
[creation_rules.proposal]
required_body = false

[creation_rules.proposal.tag.cross-cutting]
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

[revise_rules]
allow_body_revise = ["draft", "open", "working"]
allow_node_revise = ["draft", "open", "working", "review"]

[evidence_rules]
allow_evidence = ["draft", "open", "working", "review", "done", "rejected", "deprecated"]
```

#### Migration / compatibility notes

- 1.x kind-keyed policy keys (`creation_rules.rfc`, `[[guards]] on = "rfc:..."`, etc.)
  auto-rewrite to lifecycle keys at config-load time with a deprecation warning. They are
  rejected outright in 3.0 (SPEC-2.0 §10.4).
- The 1.x guard predicate `at_least_one_summary` is **no longer shipped** in 2.0 — `summary`
  is no longer a node type (ADR-006). Projects that need forced narrative content should use
  `body_sections` on `creation_rules.<lifecycle>` instead. Migration emits a warning naming
  any `policy.toml` line that still references it.

### Guard fields

- `on`: the transition that a guard block applies to, written as
  `[<facet-expr> :] from->to`. `<facet-expr>` is a boolean over `lifecycle=<value>` and
  `tag=<value>` joined by `AND` / `OR` / `NOT`. Omitting the facet expression makes the guard
  apply to all threads.
- `requires`: the list of guard rules that must pass for that transition.

#### Operation check fields

- `[checks]`: global check settings.
  - `strict`: when `true`, warnings become errors (unless `--force` is used). Default: `false`.
- `[creation_rules.<lifecycle>]` and `[creation_rules.<lifecycle>.tag.<name>]`: rules for
  creating threads. Most-specific match wins.
  - `required_body`: if `true`, the thread must have a non-empty body (Error if missing).
  - `body_sections`: list of section headings to check for in the body (Warning if missing).
- `[node_rules]`: maps state names to lists of allowed node types in that state (Error if
  violated). An absent state means all node types are allowed.
- `[revise_rules]`: controls in which states revision is allowed.
  - `allow_body_revise`: list of states where body revision is allowed (Error if violated).
  - `allow_node_revise`: list of states where node revision is allowed (Error if violated).
- `[evidence_rules]`: controls in which states evidence can be attached.
  - `allow_evidence`: list of states where evidence attachment is allowed (Error if violated).

### Guard rules currently understood by the implementation

- `no_open_objections`
- `no_open_actions`
- `one_human_approval`
- `has_commit_evidence`

### Operation checks

Operation checks validate write commands against policy rules before committing events. They are
evaluated at the CLI boundary on `new`, node commands, `revise`, and `evidence add`.

| Policy section                              | Commands checked              | What it validates |
|----------------------------------------------|-------------------------------|-------------------|
| `[creation_rules.<lifecycle>[.tag.<name>]]`  | `new`, `thread new`           | Required body, required body sections (headings) |
| `[node_rules]`                               | `comment`, `objection`, etc.  | Node type allowed in the current thread state |
| `[revise_rules]`                             | `revise`                      | Revision allowed in the current thread state |
| `[evidence_rules]`                           | `evidence add`                | Evidence addition allowed in the current thread state |

**Severity model:**

- **Error**: always blocks the operation. `--force` does NOT bypass errors.
- **Warning**: printed to stderr; operation proceeds.
  - With `strict = true` in `[checks]`: warnings become errors (blocked) unless `--force`.
  - With `--force` + `strict = true`: warnings downgrade back to warnings.

Specific severity assignments:

- Missing body when `required_body = true` → **Error**
- Missing or empty required body section → **Warning**
- Node type not allowed in state → **Error**
- Revision not allowed in state → **Error**
- Evidence not allowed in state → **Error**

**The `--force` flag:**

All write commands (`new`, node commands, `revise`, `evidence add`) accept `--force`. It bypasses
warning-level violations only. Error-level violations are never bypassed. Violations are always
printed to stderr regardless of `--force`.

**Missing or partial policy:**

- No policy file → all checks pass (no restrictions).
- Missing policy sections → no restrictions for that check (`#[serde(default)]`).

### What is enforced today

- `git forum state ...` evaluates guard rules from `[[guards]]`.
- `git forum verify` is a read-only preflight that evaluates those same guard rules without changing state.
- `git forum show` displays compact next-states with guard blockers and a state diagram.
- `git forum show --what-next` displays detailed guard checks and operation check rules for the current state.
- `git forum policy show` displays the loaded policy in human-readable format.
- `git forum policy lint` validates guard transitions and detects semantic gaps in operation allow-lists.
- All write commands evaluate operation checks from `[creation_rules]`, `[node_rules]`,
  `[revise_rules]`, and `[evidence_rules]`.

### What `git forum verify` actually does

`git forum verify` is a **preflight check**, not a history audit or integrity verifier. It is read-only — it does not change thread state or attach approvals.

It evaluates policy guards for the thread's next forward transition:

- Bug in `open` → checks guards for `open->done`
- RFC in `review` → checks guards for `review->done`
- DEC in `open` → checks guards for `open->done`
- Task in `review` → checks guards for `review->done`
- Other states → reports `ready` (no preflight target defined)

It MAY surface an advisory about the state of threads linked from the verified thread (e.g.,
"linked RFC `@1ooguji1` is not yet `done`"); this is informational only and does not affect the
verification result (SPEC-2.0 §9.4).

Use it right before an acceptance-like transition. It answers:
"If I tried to advance this thread now, which guards would block?"

### Which diagnostic command should I use?

| I want to...                                  | Command                  | Scope      |
|-----------------------------------------------|--------------------------|------------|
| See what's blocking a thread                  | `show --what-next`       | thread     |
| Check if a thread is ready to advance         | `verify`                 | thread     |
| Test guards for a specific transition         | `policy check`           | thread     |
| List unresolved objections/actions            | `status`                 | thread     |
| View the full policy rules                    | `policy show`            | repo       |
| Validate the policy file for errors           | `policy lint`            | repo       |
| Check repository health (config, index, refs) | `doctor`                 | repo       |
| Rebuild the search index                      | `reindex`                | repo       |

**Thread-scoped commands** operate on a single thread ID. **Repo-scoped commands** check the whole repository.

Quick decision tree:

1. **Something feels broken?** → `doctor` (repo health), then `reindex` if doctor suggests it.
2. **Thread won't advance?** → `show --what-next` (full picture) or `verify` (quick pass/fail).
3. **Want to test a specific transition?** → `policy check --transition from->to`.
4. **What's still unresolved?** → `status` (compact list of open items).

## Workflows

### Typical workflow

```bash
git forum init
git forum new rfc "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
# created thread @a7f3b2x1  (lifecycle=proposal, tags=cross-cutting, status=draft)

git forum comment @a7f3 "Need a stable plugin-facing boundary."
git forum comment @a7f3 "Q: what is the migration plan?" --as ai/reviewer
git forum objection @a7f3 "Benchmarks are missing."
git forum evidence add @a7f3 --kind benchmark --ref bench/result.csv
git forum comment @a7f3 "Summary: benchmarks added; objection addressed."
git forum resolve @a7f3 <OBJECTION_NODE_ID>
git forum propose @a7f3                       # draft -> open
git forum state @a7f3 review                  # open  -> review
git forum verify @a7f3
git forum accept @a7f3 --approve human/alice  # review -> done

git forum new task "Implement trait backend" --link-to @a7f3 --rel implements
# created thread @m2k9p4n8  (lifecycle=execution, tags=task, status=open)

git forum branch bind @m2k9 feat/trait-backend
git forum action @m2k9 "Wire trait backend behind feature flag."
git forum evidence add @m2k9 --kind test --ref tests/backend_trait.rs
git forum close @m2k9                         # working -> done (or open -> done if fast-tracked)
```

### AI-agent workflow

The same CLI surface works for AI agents. The typical pattern is:

1. A human or agent opens an RFC and adds initial comments.
2. An AI reviewer posts objections and questions using `--as ai/reviewer`.
3. An implementer (human or agent) replies to each objection with evidence or a comment.
4. A human resolves addressed objections, posts a summary comment, and signs the acceptance.

```bash
# 1. Human opens the RFC
git forum new rfc "Add caching layer" --body "Goal and constraints."

# 2. AI reviewer raises concerns
GIT_FORUM_ACTOR=ai/reviewer
git forum objection @<rfc-id> "No eviction strategy described."
git forum comment   @<rfc-id> "Q: what is the expected cache hit ratio?"

# 3. Implementer responds to the objection
git forum comment @<rfc-id> "LRU eviction with 10-minute TTL." \
  --reply-to <OBJECTION_NODE_ID>

# 4. Human resolves the objection, summarizes, and accepts
git forum resolve @<rfc-id> <OBJECTION_NODE_ID>
git forum comment @<rfc-id> "Summary: caching with LRU eviction approved."
git forum propose @<rfc-id>
git forum state   @<rfc-id> review
git forum accept  @<rfc-id> --approve human/alice
```

**Non-interactive body input for agents:** Since agents run without a TTY, `--edit` will
not work. Use `--body "..."` for short text, `--body-file <path>` for longer content, or
pipe through stdin with `--body -`:

```bash
echo "Detailed comment body..." | git forum comment @<rfc-id> --body -
cat /tmp/body.md | git forum revise @<rfc-id> --body -
```

## Concurrency and distribution

### Within a clone

`git-forum` uses Git's atomic ref updates (compare-and-swap) to detect concurrent writes. Each
`write_event` call reads the current thread ref tip, creates a new commit, and atomically updates
the ref only if the tip has not changed since it was read.

If two writers attempt to update the same thread simultaneously, one will succeed and the other will
fail with a clear error:

```text
concurrent write conflict on refs/forum/threads/<thread-id>: expected <sha> but ref was updated by another writer. Retry your command.
```

**Recommended patterns for parallel agent workflows:**

- **Different threads**: fully safe in parallel. Each thread has its own ref.
- **Same thread**: serialize writes, or retry on conflict. Conflicts are rare for human workflows
  but more likely when multiple agents update the same thread simultaneously.
- **Create vs update**: thread creation uses `create_ref` which fails if the ref already exists,
  preventing duplicate thread IDs.

### Across clones

Forum data lives in `refs/forum/*` and is replicated with standard `git push` and `git fetch`
on those refs. `git-forum` does not introduce its own push / fetch protocol or cross-clone
conflict resolution (SPEC-2.0 §8.2; CORE-VALUE.md non-goal §3).

When a non-fast-forward push fails, resolve it with the standard Git workflow (fetch, rebase or
merge the affected forum refs, re-push). `git forum doctor` reports any divergence visible in
the local refs **informationally** — it does not enforce reconciliation.

`git forum init` configures a `refs/forum/*:refs/forum/*` fetch refspec on the `origin` remote
so that subsequent clones pick up forum data automatically.

## Per-subcommand help targets

For focused reference on a specific topic, use per-subcommand `--help-llm`:

```
git forum comment --help-llm     Node type taxonomy (4 canonical types)
git forum state --help-llm       Unified state machine and lifecycle filter
git forum evidence --help-llm    Evidence kinds reference (8 kinds)
```

Per-subcommand `--help-llm` prints a focused ~200-token reference section instead of the full
manual. All node shorthand commands (`comment`, `objection`, `action`, `node`, plus the
deprecated `claim` / `question` / `summary` / `risk` / `review` / `alternative` /
`assumption` aliases) print the node type taxonomy. All state commands (`state`, `close`,
`pend`, `accept`, `propose`, `reject`, `withdraw`, `deprecate`) print the unified state
machine and lifecycle filter. `evidence` prints the evidence kinds reference. All other
subcommands print this full manual.

## Setup and tools

### Install

```bash
cargo install --path .
git-forum --help
```

If you only want to try it during development:

```bash
cargo run -- --help
```

**Note:** `git forum --help` requires `git forum init` to have been run first.
`init` sets a local git alias (`alias.forum = !git-forum`) so that `--help` is
passed to the binary instead of triggering Git's man-page lookup.

### Repository setup

Initialize `git-forum` inside a repository before using it:

```bash
git forum init
git forum doctor
git forum reindex
```

- `init`: creates `.forum/` and `.git/forum/`, installs git-forum hooks (commit-msg and post-checkout), configures a `refs/forum/*` fetch refspec on `origin`.
- `doctor`: checks `.forum/` and `.git/forum/` directories exist, validates `policy.toml` syntax, verifies template files are present and non-empty, checks SQLite index health (integrity and freshness), checks for missing blob references in the git index, replays every thread's event log to verify integrity, and reports any divergence visible in local refs informationally. Reports `[ok]`, `[WARN]`, or `[FAIL]` per check; exits non-zero only on failures (warnings and advisories are informational).
- `reindex`: rebuilds the local index from Git refs.

#### Migrating from 1.x

```bash
git forum migrate --dry-run
git forum migrate
```

`migrate` is a one-shot rewrite of a 1.x repo into the 2.0 storage format
(SPEC-2.0 §10):

- thread refs move from `refs/forum/threads/<KIND>-<token>` to
  `refs/forum/threads/<token>`; the old name is preserved as a read-only alias
  so external links keep resolving;
- each thread gets a `facet_set` event populating `lifecycle` and the
  conventional tags (`cross-cutting` for `rfc`; `bug` for `issue`/`ask`; `task`
  for `task`/`job`);
- 1.x states are remapped to the unified state set (`proposed → open` for
  RFCs, `under-review → review`, `accepted → done`, `closed → done` for
  execution and record threads, etc.);
- 1.x node types `claim` / `question` / `summary` / `risk` / `review` /
  `alternative` / `assumption` are rewritten to `comment` with the original
  type preserved in `legacy_subtype`; standalone Approval events become
  `approval` nodes; `objection`, `action`, and `evidence` are unchanged.
- existing thread-to-thread links are preserved; they are the only grouping
  mechanism in 2.0.

#### Local configuration

Per-clone settings live in `.git/forum/local.toml` (never committed). This file is optional;
defaults apply when it is absent.

#### Commit identity

By default, forum commits use your Git config `user.name` and `user.email` as the commit
author/committer. To override this (e.g., for privacy when pushing forum refs to a remote),
add a `[commit_identity]` section:

```toml
# .git/forum/local.toml
[commit_identity]
name = "alice"
email = "alice@example.com"
```

Both fields are optional. Unset fields fall through to the Git config defaults. This controls
only the Git commit metadata (author/committer); the forum actor ID (`human/alice`) in
`event.json` is controlled separately via `--as` or `GIT_FORUM_ACTOR`.

### Hooks

`git forum init` automatically installs two git hooks:

```bash
git forum hook install              # install all git-forum hooks
git forum hook install --force      # overwrite existing hooks (no backup)
git forum hook uninstall            # remove all git-forum hooks
```

#### Commit-msg hook

An advisory hook that validates thread ID references in commit messages. Delegates to
`git-forum hook check-commit-msg <file>`, which:

1. Strips Git comment lines (respecting `core.commentChar`) and scissors sections.
2. Scans the cleaned message for thread ID patterns: 2.0 native (`@XXXXXXXX` and bare 8-char
   base36 tokens) and legacy 1.x (`KIND-XXXXXXXX` opaque, `KIND-NNNN` sequential).
3. Validates each referenced thread exists in `refs/forum/threads/`.

**Behavior:**

- No thread IDs found: prints a warning, exits 0 (commit proceeds).
- All referenced threads exist: exits 0 silently.
- Any referenced thread missing: prints a warning with the missing IDs, exits 1 (commit blocked).

```text
git-forum: commit message references non-existent thread(s):
  @9999z9z9 — not found
hint: create the thread first, or remove the reference from the commit message.
```

#### Post-checkout hook

Runs after `git checkout`, `git switch`, and `git worktree add`. Performs two actions:

1. **`git-forum hook worktree-init`** — auto-initializes git-forum in new worktrees (creates
   `.git/forum/`, `local.toml`, installs hooks). No-op if already initialized.
2. **`git-forum hook fix-index`** — repairs missing blob references in two places: (a) the
   git index, by re-hashing working-tree copies of any staged paths whose blob is missing;
   (b) HEAD's tree, by re-staging paths whose HEAD blob is missing so the next commit lands
   a fresh tree. The mechanism by which an index or HEAD-tree blob goes missing under normal
   use is not characterized; `fix-index` exists as defense-in-depth, not because a specific
   git operation is known to produce the corruption. When the symptom does occur, the
   recovery flow is `git-forum hook fix-index && git commit --no-verify` — `--no-verify` is
   unavoidable for one commit because pre-commit's startup `git diff --diff-filter=A` probe
   dies on the broken HEAD before any user hook can run.

#### Hook path resolution

Hook paths are resolved via `git rev-parse --git-path hooks/<name>`, so they work correctly
with worktrees and `core.hooksPath`. `--force` overwrites any existing hook without backup; users
with custom hooks should use a hook dispatcher (e.g., the pre-commit framework).

### Purge (hard-delete)

`git forum purge` permanently removes event content from git history by rewriting commits.
This is destructive: commit SHAs change and all clones must re-fetch affected refs.

#### Purge a specific event

```bash
git forum purge --thread @<thread-id> --event <SHA>
git forum purge --thread @<thread-id> --event <SHA> --dry-run
```

Replaces the event's `body` and `title` with `[purged]`. All downstream commits in the thread
are rewritten with new parent SHAs. The SQLite index is rebuilt automatically.

#### Purge all events by an actor

```bash
git forum purge --actor human/alice
git forum purge --actor human/alice --dry-run
```

Replaces the `actor`, `body`, and `title` with `[purged]` on every event created by the
specified actor, across all threads. Use `--dry-run` first to review what would be affected.

#### After purging

- Commit SHAs change for all rewritten commits. Push with `git push --force-with-lease`.
- All clones must re-fetch affected refs (`git fetch origin refs/forum/*:refs/forum/*`).
- Original objects remain in `.git/objects/` until `git gc` prunes them.
- Run `git gc --prune=now` locally to remove unreachable objects immediately.

## TUI

```bash
git forum tui
git forum tui @<rfc-id>
```

### Display surface (SPEC-2.0)

- **Thread IDs**: bare 8-char base36 IDs render with a leading `@` marker
  (e.g. `@a7f3b2x1`). Legacy kind-prefixed IDs (`RFC-…`, `ASK-…`, `DEC-…`,
  `JOB-…`) render unchanged. The marker is display-only — every CLI surface
  also accepts the bare token without `@`.
- **Thread detail header**: shows `lifecycle`, `tags`, and `status` instead
  of the 1.x `kind`. Unmigrated 1.x threads (no `facet_set` event) display
  the conventional tag derived from kind per SPEC-2.0 §2.3.3 (`rfc` →
  `cross-cutting`, `issue` → `bug`, `task` → `task`); no event is written.
- **List view**: column header is `LIFECYCLE` (was `KIND`). Tags render as
  a leading bracket prefix on the title cell when non-empty, e.g.
  `[bug] search is slow`.
- **Linked panel**: thread detail shows a one-line "linked" advisory below
  the body. Pure display, no enforcement (CORE-VALUE.md "Advisories").

### Colors

The TUI uses color to distinguish lifecycle, statuses, and node types:

- **Thread lifecycle**: cyan = `proposal`, yellow = `execution`, magenta =
  `record`.
- **Thread status**: green = `open`/`draft`, yellow = `working`/`review`,
  magenta = `done`, red = `rejected`, gray = `deprecated`/`withdrawn`.
- **Node type**: red = `objection`, green = `approval`, cyan = `action`,
  default = `comment`. Pre-existing nodes carrying legacy prose-only types
  (claim, question, summary, risk, review, alternative, assumption) render
  with their stored label and the default colour.
- **Node status**: green = open, gray = resolved/retracted/incorporated.

Resolved, retracted, and incorporated node rows are dimmed.

### Controls

- **List view**: `j`/`k` navigate, `enter` opens thread, `c` creates, `f`
  cycles the lifecycle / tag / status filter, `r` refreshes, `q` quits.
  Click column headers to sort; click/double-click rows to select/open.
- **Thread detail**: `j`/`k` navigate nodes, `up`/`down` scroll body,
  `enter` opens node, `c` creates node, `l` creates link, `m` toggles
  markdown, `S` enters select mode, `r` refreshes, `esc`/`q` goes back.
- **Node detail**: `c` creates node, `l` creates link, `x` resolves, `o`
  reopens, `R` retracts, `m` toggles markdown, `j`/`k` scroll, `r`
  refreshes, `esc`/`q` goes back.
- **Create thread form**: fields are `lifecycle`, `tags`, `title`, `body`.
  `tab` moves between fields; `up`/`down` cycle the lifecycle on the
  `lifecycle` field. The `tags` field accepts a comma-separated list and
  is validated against the SPEC-2.0 §2.3.5 grammar at submit time. When
  `(lifecycle, tags)` matches one of the four §9.1 kind presets the form
  emits the preset Event shape; otherwise it writes a `facet_set` event
  carrying the chosen tag list. Tag editing of existing threads is
  CLI-only in 2.0.
- **Create node form**: defaults to `comment`. Available types are the
  four canonical 2.0 types: `comment`, `approval`, `objection`, `action`.
- **Body editor**: `enter` on body opens editor, `enter` on submit creates,
  `ctrl+s` in body editor returns to form, `esc` cancels.
