# Manual

> **3.0 vocabulary.** A thread carries a single required **category**
> (`rfc` or `task`) plus free-form **tags**. The four 1.x kind presets
> (`rfc`, `dec`, `task`, `issue`/`bug`) live on as top-level shortcuts
> that map to a (category, tag) pair (SPEC-3.0 §2.4 / §8.3). Discussion
> uses four canonical node kinds — `comment`, `approval`, `objection`,
> `action` — chosen by protocol effect (SPEC-3.0 §2.2 / ADR-006).
> Thread states are unified across categories: `draft`, `open`,
> `working`, `review`, `done`, `rejected`, `withdrawn`, `deprecated`
> (SPEC-3.0 §3.1). Thread IDs display as `@XXXXXXXX` and store as the
> bare 8-char base36 token (SPEC-3.0 §6).
>
> Storage is a **snapshot tree** at `refs/forum/threads/<id>`:
> `thread.toml`, optional `body.md`, `nodes/<id>.{toml,md}`,
> `links.toml`, `evidence.toml` (SPEC-3.0 §4.2). Each write creates
> a new commit on the thread ref; revision history is `git log` over
> the ref. There is no event chain, no index sidecar, and no
> snapshot/log distinction.
>
> Repos written under 1.x or 2.x event chains are read via
> `git forum migrate --to 3.0`, which projects the legacy chain
> into the 3.0 snapshot tree once. Any non-migrate command that
> encounters a legacy ref bails with `LegacyEventChain` and asks
> the operator to migrate first.

## Quick Reference

```text
# create — kind preset (everyday)
git forum new <kind> "Title" [--body "..."|--edit]   Create via kind preset
                                                     (rfc/dec/task/issue/bug)
                                                     → maps to (category, tags)

# create — canonical (power-user, scriptable; SPEC-3.0 §7)
git forum thread new "Title" --category <C> [--tag <T>]...
                                                     Create with explicit
                                                     category + tags

# inspect
git forum ls [--kind <kind>] [--status <S>] [--branch <B>]
                                                     List threads
git forum show <ID>                                  Show thread details
git forum show <ID> --what-next                      Show valid next actions
git forum show <ID> --compact                        Compact single-line view
git forum show <ID> --no-timeline                    Omit timeline
git forum show <ID> --tree                           Show direct `implements`
                                                     children (advisory)
git forum brief <ID> [--json]                        Read-only digest
git forum diff <ID> [--rev N|N..M]                   Diff body revisions
git forum shortlog --since <DATE_OR_REV>             Threads resolved since
git forum status <ID>                                Open items for a thread
git forum node show <NODE_ID>                        Inspect a node

# discussion (canonical 3.0)
git forum node add <ID> --type <type> "body"         Add a typed node
git forum comment <ID> "body"                        node add --type comment
git forum objection <ID> "body"                      node add --type objection
git forum action <ID> "body"                         node add --type action
git forum revise body <ID> --body "..."              Revise thread body
git forum revise node <NODE> --body "..."            Revise a node body
git forum retype <ID> <NODE> --type <new_type>       Change node kind
git forum retract <ID> <NODE>...                     Soft-delete nodes
git forum resolve <ID> <NODE>...                     Mark node addressed
git forum reopen <ID> [<NODE>...]                    Reopen node or thread

# state transitions
git forum state <ID> <NEW_STATE>                     Generic transition
git forum state bulk --to <STATE> <ID>...            Bulk transition
git forum close <ID>                                 task → done | rfc rejects
git forum accept <ID>                                rfc/record → done
git forum propose <ID>                               rfc draft → open
git forum pend <ID>                                  task → working
git forum reject <ID>                                any → rejected
git forum withdraw <ID>                              rfc → withdrawn
git forum deprecate <ID>                             any → deprecated

# evidence and links
git forum evidence add <ID> --kind <K> --ref <R>...  Attach evidence
git forum link <ID> <TARGET> --rel <REL>             Cross-thread link
git forum branch bind <ID> [<BRANCH>]                Bind branch scope
git forum branch clear <ID>                          Clear bound branch

# policy and preflight
git forum verify <ID>                                Preflight: forward target
git forum policy show                                Show loaded policy
git forum policy lint                                Validate policy.toml
git forum policy check <ID> --to <STATE>             Re-evaluate guards

# repo health and migration
git forum doctor [--strict] [-v]                     Diagnose repo
git forum migrate [--dry-run]                        1.x/2.x → 3.0

# hooks and TUI
git forum hook install                               Install commit-msg
git forum hook uninstall                             Remove hooks
git forum tui [<ID>]                                 Open the TUI
```

## Conventions

**Category.** Every thread has exactly one `category`, one of two
built-in registry entries — `rfc` (proposal-style review) or `task`
(execution-style work). The category determines the initial `status`
and the policy guard set (SPEC-3.0 §3.1). Policy MAY add custom
categories via `policy.toml`; built-in commands always recognize
`rfc` and `task`.

**Tags.** Free-form `tag = ["..."]` array on `thread.toml`. Tags are
how a `task` thread distinguishes between an everyday work item
(`["task"]`), a bug report (`["bug"]`), or a decision record
(`["decision"]`). The `decision` tag is canonical (SPEC-3.0 §8.3) —
it is what `git forum new dec` writes, and what migration uses to
preserve dec/record classification when collapsing the v2 kind axis
to the 3.0 category axis.

**Status.** A thread's lifecycle position. The unified state machine
covers `draft → open → working → review → done`, plus the terminal
branches `rejected`, `withdrawn`, `deprecated`. The shorthand commands
(`close`, `accept`, `propose`, `pend`, `reject`, `withdraw`,
`deprecate`) are category-aware: `close` on an `rfc` rejects (use
`accept`); `propose` on a `task` rejects (proposal-only).

**Node kinds.** Four canonical kinds, chosen by protocol effect:
`comment` (informational), `approval` (advances policy guards),
`objection` (blocks forward transitions while open), `action`
(blocks forward transitions while open if the policy
`NoOpenActions` guard is in effect). Per SPEC-3.0 §2.2 and ADR-006
the rhetorical 1.x shorthands (`claim`, `question`, `summary`,
`risk`, `review`) are no longer node kinds in 3.0; they survive in
migrated threads as a `legacy_label` on `comment` nodes.

**IDs.** Thread IDs are 8-char base36 (e.g. `1hg98odf`). Display
form is `@<id>`. Node IDs are 16-char base36 (longer to avoid
intra-thread collisions). Both are content-derived (actor + body
+ timestamp; SPEC-3.0 §6).

**Storage.** Each thread is a Git tree at `refs/forum/threads/<id>`:

```text
thread.toml          metadata: id, title, category, status, tags,
                     branch, created_*, updated_*, supersedes
body.md              optional thread body (Markdown)
nodes/
  <node_id>.toml     per-node metadata
  <node_id>.md       per-node body
links.toml           outgoing thread-to-thread edges
evidence.toml        attached commit/PR/file evidence
```

Mutations rewrite affected files and create a new commit on the
thread ref. There is no separate event log; revision history is
`git log` over the ref. The optional `refs/forum/index/*` namespace
is a rebuildable cache only (SPEC-3.0 §9.2).

**Refs trailer.** Connect a code commit to one or more threads with
a `Refs:` trailer in the commit message:

```text
Add JWT validator

Implements the auth layer.

Refs: @1hg98odf
```

`git forum hook install` adds the `commit-msg` validator hook that
checks `Refs:` trailers point at known threads. The hook also
attaches each `Refs:` thread as `kind=commit` evidence on the next
`evidence add`. There is no `Threads:` or `Touches:` legacy form.

### Trust model

`git forum` is fundamentally a Git wrapper. Threads are just files
on a Git ref, mutations are commits, and history is `git log`. The
tool offers:

- a structured **schema** (categories, statuses, node kinds);
- a **policy engine** (transition guards, operation checks);
- a **CLI** for the everyday surface;
- a **TUI** for richer review.

The tool does **not** replace your code review system, ticket tracker,
or chat — it exists alongside them. Threads live where the code lives,
which is what makes `Refs:` trailers and `branch bind` useful.

## Picking a kind preset

The 5 built-in presets map to a `(category, tags)` pair:

| preset  | category | tags        | initial status | typical use |
|---------|----------|-------------|----------------|-------------|
| `rfc`   | `rfc`    | `[]`        | `draft`        | proposal under review |
| `dec`   | `task`   | `[decision]`| `open`         | decision record |
| `task`  | `task`   | `[task]`    | `open`         | everyday work item |
| `issue` | `task`   | `[bug]`     | `open`         | bug report |
| `bug`   | `task`   | `[bug]`     | `open`         | alias for `issue` |

Use `git forum thread new --category <C> --tag <T>...` for the
canonical form, or `git forum new <preset>` for the everyday one.

## Create threads

### RFC (`category=rfc`)

```text
git forum new rfc "Replace JWT validation with OPA" --body "..."
git forum new rfc "Search index ranking refresh" --edit
```

RFCs start in `draft`. Move to `open` (review-ready) with
`git forum propose <ID>`, advance to `done` with
`git forum accept <ID>` (which checks `OneHumanApproval` guards).

The `policy.toml` guard set MAY require a body and at least one
human approval before `done`; defaults are conservative. Run
`git forum verify <ID>` to see which guards block forward transitions.

### DEC (`category=task`, `tag=decision`)

```text
git forum new dec "Adopt JWT-libv2 over OPA-go" --body "..."
```

Decision records start in `open` and are accepted via
`git forum accept <ID>` (record category transitions
`open → done`). The canonical `decision` tag is added automatically
so `ls --tag decision` always finds them.

### Task (`category=task`, `tag=task`)

```text
git forum new task "Wire up cache eviction"
git forum new task "Refactor account adapter" --edit
```

Tasks start in `open`. Move to `working` when picked up
(`git forum pend <ID>`) and to `done` with `git forum close <ID>`.

### Issue / bug (`category=task`, `tag=bug`)

```text
git forum new issue "Login redirect drops query string"
git forum new bug "Cache eviction races on shutdown"
```

Issues are tasks tagged `bug`. The shorthand `git forum close`
moves them to `done`; `git forum reject` records that the issue
is invalid or won't be fixed.

### Inline nodes at creation

A new thread can carry inline nodes from the start:

```text
git forum new rfc "Replace JWT validator" \
  --body "..." \
  --objection "Risks lockout if libv2 is mis-configured" \
  --action "Audit existing JWT key-rotation code" \
  --action "Document rollback path"
```

Each `--objection` / `--action` value writes a `nodes/<id>.{toml,md}`
pair at thread-creation time. The legacy `--claim`, `--question`,
`--risk`, `--summary` flags still parse, but the resulting nodes
write as `comment` kind with a `legacy_label` field — they no longer
mean anything to policy guards.

### Create from a commit

```text
git forum new task --from-commit HEAD
git forum new issue --from-commit abc1234 --edit
```

The thread's title and body seed from the commit message (subject
line → title; body → body). The commit SHA is recorded as
`evidence.toml` `kind=commit`, so `git forum show` and policy
guards (e.g. `HasCommitEvidence`) see it immediately.

### Create from another thread

```text
git forum new rfc --from-thread @abcdef01 --body "v2 of the proposal"
```

Records a `supersedes` row in `thread.toml` and writes the symmetric
`superseded-by` link on the source thread. The source MUST already be
a 3.0 snapshot — if it's still on a 1.x/2.x event chain, the command
bails with `LegacyEventChain` before any write happens. Run
`git forum migrate --to 3.0` first.

An execution thread cannot supersede a proposal thread (SPEC-3.0
§9.3): use `git forum link <NEW> <SOURCE> --rel implements` instead.

### Filter the thread list

```text
git forum ls
git forum ls rfc                       # positional kind shorthand
git forum ls --kind issue
git forum ls --status open
git forum ls --branch main             # threads bound to branch=main
```

The `ls` reader walks `refs/forum/threads/*` and reads `thread.toml`
from each ref tip. There is no index dependency; if `--kind` filters
recover legacy 1.x kinds, the kind→(category, tags) translation
follows SPEC-3.0 §8.3.

## Structured discussion

### Add a node

```text
git forum comment @1hg98odf "Worth checking the libv2 changelog"
git forum objection @1hg98odf "This breaks downstream signing"
git forum action @1hg98odf "Audit existing key-rotation paths"
git forum node add @1hg98odf --type comment "..."
```

All three shorthand commands (`comment`, `objection`, `action`) are
aliases for `node add --type <kind>`. There is no `approval` shorthand
(approvals are recorded by state-change commands' `--approve <ACTOR>`
flag and by hook validators).

Each node writes:

```text
nodes/<node_id>.toml      kind, status, created_*, updated_*, reply_to
nodes/<node_id>.md        node body (Markdown)
```

`thread.toml.updated_*` is bumped to the current actor + timestamp
in the same commit.

### Retype a node

Mistaken kind? Re-classify in place:

```text
git forum retype @1hg98odf <NODE> --type action
```

The retype rewrites only `nodes/<id>.toml` `type` field (not the
body), commits to the thread ref. There is no event-chain
"retype" entry — `git log` over the snapshot ref shows the change.

### Revise a node

```text
git forum revise node <NODE> --body "Clarified: only affects libv1 callers"
git forum revise node <NODE> --edit
```

Overwrites `nodes/<id>.md`. The `nodes/<id>.toml` updated_at /
updated_by fields move to the current actor; previous body is
in `git log` over the ref.

### Retract / resolve / reopen a node

```text
git forum retract @1hg98odf <NODE>...      # soft-delete (still readable)
git forum resolve @1hg98odf <NODE>...      # mark addressed
git forum reopen  @1hg98odf <NODE>...      # back to open
```

These set `nodes/<id>.toml.status` to `retracted` / `resolved` /
`open`. Bodies are unchanged. Open `objection` and `action` nodes
block forward state transitions; resolving them clears the policy
guard.

`git forum reopen <ID>` (no node IDs) reopens the thread itself —
sets `thread.toml.status` back to `open` from a closed state.

### Reply to a node

```text
git forum comment @1hg98odf "Agreed, see RFC-2.5" --reply-to <NODE>
```

The new node's `reply_to` field points at the parent. The TUI
shows replies threaded; CLI `show` displays them indented.

### Inspect a single node

```text
git forum node show <NODE>
```

Walks every thread ref, finds the node, prints metadata + body +
the parent thread's link table. Uses unique-prefix matching: 8+
characters or any exact match.

## State transitions

### Status

Run `git forum show <ID>` to see the current status, the valid
next states, and any guard conditions blocking forward motion.

### The unified state machine

3.0 unifies category state machines into a single graph:

```text
            draft  →  open  →  working  →  review  →  done
              ↘       ↘        ↘            ↘         ↑
                rejected  ←————————————————————┘ (re-open from rejected)
                       ↘
                        deprecated, withdrawn (terminal)
```

Per-category restrictions:

- **rfc**: starts in `draft`; valid transitions are
  `draft → open → review → done` plus `→ withdrawn` (any non-terminal)
  and `→ rejected`. `accept` is the conventional shorthand for
  `→ done`; `propose` is `draft → open`.
- **task**: starts in `open`; valid transitions are
  `open → working → done` plus `→ rejected`. `pend` is the
  conventional shorthand for `→ working`; `close` is `→ done`.

`git forum verify <ID>` checks the next forward target against
policy guards and reports which (if any) block the move.

### Shorthand commands

| command     | rfc                | task           |
|-------------|--------------------|----------------|
| `close`     | rejected (use accept) | → done       |
| `accept`    | → done             | rejected (use close) |
| `propose`   | draft → open       | rejected       |
| `pend`      | rejected           | → working      |
| `reject`    | → rejected         | → rejected     |
| `withdraw`  | → withdrawn        | rejected       |
| `deprecate` | → deprecated       | → deprecated   |

Each shorthand accepts the same flags as `state`: `--as`,
`--approve`, `--resolve-open-actions`, `--link-to`, `--rel`,
`--comment`, `--fast-track`, `--force`.

### Generic state command

```text
git forum state @1hg98odf done              # transition
git forum state @1hg98odf review --comment "ready for review"
git forum state @1hg98odf done --approve human/alice --approve human/bob
git forum state bulk --to done @abc @def @ghi
```

`--fast-track` walks intermediate states automatically (e.g. a
`task` in `open` → `done` walks `open → working → done` in a
single command, recording each commit on the ref).

`--resolve-open-actions` resolves any open `action` nodes inline
so the `NoOpenActions` guard passes; equivalent to running
`git forum resolve <ID> <each_action>` then the state change.

`--link-to <THREAD>` plus `--rel <REL>` records a link in the same
commit as the state change — useful for capturing the implementing
PR thread when closing an RFC.

## Browse and inspect

### List threads

```text
git forum ls                             # everything
git forum ls --kind rfc
git forum ls --status open
git forum ls --branch feature/auth
```

The list is sorted by `thread.toml.updated_at` descending. Each
row shows ID, status, category/tags, and title. Use `git forum show
<ID>` for full detail.

### Show thread details

```text
git forum show @1hg98odf
git forum show @1hg98odf --what-next
git forum show @1hg98odf --compact
git forum show @1hg98odf --no-timeline
git forum show @1hg98odf --tree
```

The default rendering reads the snapshot tree and prints:

- header (id, title, category, tags, status, branch);
- valid next transitions (status diagram);
- thread body (if present);
- nodes (resolved + retracted dimmed);
- links (outgoing edges from `links.toml`);
- evidence (rows from `evidence.toml`);
- timeline (a Git-log-over-snapshot view: who changed what when).

`--what-next` adds a per-guard report (which guards pass, which
block, and the actor IDs needed to satisfy them). `--tree` lists
direct incoming `implements` children — useful for an RFC to
discover its execution threads.

### Brief

```text
git forum brief @1hg98odf
git forum brief @1hg98odf --json
```

A read-only single-thread digest aimed at LLMs and review tools:
status, open objections, open actions, evidence count, link
counts. JSON output is stable across versions.

### Diff body revisions

```text
git forum diff @1hg98odf                 # latest body change
git forum diff @1hg98odf --rev 3         # rev 2 vs 3
git forum diff @1hg98odf --rev 1..3      # rev 1 vs 3
```

Diff is a `git diff` over `body.md` between two commits on the
ref. There is no separate revision counter in 3.0 — `--rev N`
indexes the N-th commit on the ref that modified `body.md`.

### Shortlog

```text
git forum shortlog --since 2026-01-01
git forum shortlog --since v1.4.0
git forum shortlog --since 2026-01-01 --kind rfc
```

Lists threads that reached a terminal state (`done`, `rejected`,
`withdrawn`, `deprecated`) since the given date or revision. The
terminal-state-date check reads the thread ref tip's commit time;
no event chain is walked.

### Status (open items)

```text
git forum status @1hg98odf
```

Compact view: open objections, open actions, missing evidence,
unsatisfied guards. Equivalent to `show --what-next` filtered to
the unresolved subset.

## Evidence and links

### Add evidence to a thread

```text
git forum evidence add @1hg98odf --kind commit --ref HEAD
git forum evidence add @1hg98odf --kind commit --ref a1b2c3d --ref e4f5
git forum evidence add @1hg98odf --kind file --ref src/auth/jwt.rs
git forum evidence add @1hg98odf --kind test --ref tests/jwt_test.rs
git forum evidence add @1hg98odf --kind benchmark --ref bench/result.csv
git forum evidence add @1hg98odf --kind external --ref https://example.com/postmortem
```

Each `--ref` writes one row to `evidence.toml`. The supported
kinds are `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`,
`thread`, `external`. For `kind=commit`, the ref string is
canonicalized through `git rev-parse` before storing — `--ref HEAD`
becomes the resolved 40-char SHA.

### Linking implementation commits

The `commit-msg` hook recognizes `Refs: @<id>` trailers and
auto-attaches the commit as evidence on the next `evidence add`.
For the everyday flow you can skip the manual `evidence add`:

```text
$ git commit -m "Add JWT validator

Refs: @1hg98odf"
```

The hook validates that the referenced thread exists; the
post-checkout hook handles worktree initialization.

### Link two threads

```text
git forum link @1hg98odf @abcdef01 --rel implements
git forum link @1hg98odf @abcdef01 --rel blocks
git forum link @1hg98odf @abcdef01 --rel related
```

Writes one row to `links.toml` on the FROM thread. The link is
one-way; if you want a reverse edge run `link` again with the
arguments swapped. Built-in relations: `implements`, `blocks`,
`related`, `supersedes` (and the inverse `superseded-by`,
written automatically by `new --from-thread`).

### Bind a thread to a Git branch

```text
git forum branch bind @1hg98odf feature/jwt-rewrite
git forum branch bind @1hg98odf                        # binds to current branch
git forum branch clear @1hg98odf
```

Sets `thread.toml.branch` to the named branch (or the current
branch when `<BRANCH>` is omitted). The branch must exist at
bind time. Bound branches surface in `ls --branch` filtering and
in the branch column of `show` output.

## Body revision

### Revise thread body

```text
git forum revise body @1hg98odf --body "..."
git forum revise body @1hg98odf --body-file ./body.md
git forum revise body @1hg98odf --edit
git forum revise @1hg98odf --body "..."                # default form
```

Overwrites `body.md` on the thread ref. Previous bodies are
visible via `git log` and `git forum diff <ID>`.

`--incorporates <NODE>` marks one or more nodes as incorporated
into this revision — they appear in `show` as resolved with an
"incorporated by revise-body" tag.

### Diff body revisions

See "Browse and inspect → Diff body revisions" above. Revisions
are just commits on the ref that modified `body.md`.

## Preflight and policy

### What is enforced today

3.0 reuses the 2.0 policy engine. `policy.toml` defines:

- **transition guards** — preconditions for a state transition
  (e.g. `OneHumanApproval`, `NoOpenObjections`, `HasCommitEvidence`);
- **operation checks** — preconditions for a mutation other than
  state change (e.g. `RequiredBody` on thread creation).

### The policy file

`./.forum/policy.toml` is the per-repo policy. A minimal example:

```toml
[checks]
strict = false                            # warn-vs-error mode

[[guards]]
scope    = "category=rfc;status=review->done"
rules    = ["OneHumanApproval", "NoOpenObjections"]

[[guards]]
scope    = "category=task;status=working->done"
rules    = ["NoOpenActions", "NoOpenObjections"]

[creation_rules.rfc]
required_body = true
```

Guard scopes use the `category=...;status=FROM->TO` form. The 1.x
`kind:from->to` form still parses but emits a deprecation warning
on load; `git forum migrate` rewrites the file in-place during
migration.

### Guard rules currently understood by the implementation

- `NoOpenObjections` — every `objection` node must be resolved or
  retracted before the transition.
- `NoOpenActions` — every `action` node must be resolved or
  retracted before the transition.
- `OneHumanApproval` — at least one `human/*` actor has called
  `state ... --approve <ACTOR>` (or has been recorded via the
  TUI approval flow).
- `HasCommitEvidence` — at least one `evidence.toml` row has
  `kind=commit`.

### Guard fields

Each `[[guards]]` table accepts:

- `scope` (string, required) — `category=...;status=FROM->TO`.
- `rules` (array, required) — guard names from the list above.
- `requires_actor_role` (string, optional) — restrict the
  satisfying actor's role (e.g. `lead`).

### Operation checks

Per-category creation overrides:

```toml
[creation_rules.rfc]
required_body = true
required_tag  = ["domain"]

[creation_rules.task]
required_body = false
```

The `--force` flag on creation/state commands bypasses
warning-level checks (does not bypass errors). Set `[checks]
strict = true` to promote warnings to errors globally.

### Verify (preflight)

```text
git forum verify @1hg98odf
```

Evaluates the guards for the thread's next forward transition
without changing state. Reports each guard's name, whether it
passes, and the actor/evidence required if blocked.

### Policy sub-commands

```text
git forum policy show                    # human-readable dump
git forum policy lint                    # validate policy.toml syntax
git forum policy check @ID --to <STATE>  # re-run guards for a target
```

`policy lint` is run automatically by `doctor`; the standalone
form is useful in CI.

### Which diagnostic command should I use?

| symptom | command |
|---------|---------|
| "Why is `accept` blocked?" | `git forum verify <ID>` |
| "What's left on this thread?" | `git forum status <ID>` |
| "Is policy.toml valid?" | `git forum policy lint` |
| "Is this repo healthy?" | `git forum doctor` |
| "Are any threads still on the legacy event chain?" | `git forum doctor --strict` |

## Workflows

### Typical workflow

```text
# 1. Open an RFC for the change
git forum new rfc "Replace JWT validator" --edit

# 2. Discuss
git forum objection @ABCDEF01 "Risks libv1 callers"
git forum comment   @ABCDEF01 "Mitigation: feature-flag the rollout"

# 3. Resolve objections, then propose for review
git forum resolve @ABCDEF01 <objection_node>
git forum propose @ABCDEF01

# 4. Get approvals and accept
git forum accept @ABCDEF01 --approve human/alice --approve human/bob

# 5. Open implementation tasks
git forum new task "Wire up JWT validator" \
  --from-thread @ABCDEF01 --link-to @ABCDEF01 --rel implements

# 6. Commit code with a Refs trailer
git commit -m "Implement JWT validator

Refs: @<task_id>"

# 7. Close the task
git forum close @<task_id>
```

### AI-agent workflow

When `git forum` is driven by an AI agent (`--as ai/<name>`), the
recommended pattern is:

1. **Read first** — `git forum show <ID>` or `git forum brief <ID>
   --json` to load thread state into the agent's context.
2. **Reply, don't restart** — use `--reply-to <NODE>` so threads
   stay coherent.
3. **One commit per concern** — split node-add and state-change
   into separate `git forum` invocations; both commit to the
   thread ref, and `git log` becomes the audit trail.
4. **Use `--as ai/<name>`** consistently so policy guards that
   require a `human/*` approval don't accept the agent's own
   approvals.

### End-to-end scenario: search-perf RFC

```text
$ git forum new rfc "Search index ranking refresh" \
    --body-file ./rfc.md \
    --tag domain=search

Created @9p3v2k7t

$ git forum show @9p3v2k7t --what-next

[ guard checks for draft → open ]
- RequiredBody         PASS
- AtLeastOneTag        PASS

$ git forum propose @9p3v2k7t
@9p3v2k7t: draft → open

$ git forum objection @9p3v2k7t \
    "Index rebuild downtime is unacceptable" \
    --as human/sre

$ git forum action @9p3v2k7t \
    "Benchmark warm-rebuild path on staging" \
    --as human/lead

$ git forum show @9p3v2k7t --what-next

[ guard checks for review → done ]
- NoOpenObjections     BLOCK (1 open: <id>)
- NoOpenActions        BLOCK (1 open: <id>)
- OneHumanApproval     BLOCK (no human/* approval recorded)

$ git forum resolve @9p3v2k7t <objection_id> <action_id>

$ git forum state @9p3v2k7t review

$ git forum accept @9p3v2k7t --approve human/sre --approve human/lead
@9p3v2k7t: review → done
```

## Concurrency and distribution

### Within a clone

Each mutation reads the current ref tip, applies the edit, and
writes a new commit with `update-ref` CAS. If another writer
got there first, the CAS fails and the command returns a
`ConcurrentWrite` error — re-run after re-reading. The TUI
handles this transparently for same-thread retries.

### Across clones

Forum refs (`refs/forum/threads/*`) are pushed and fetched like
any other Git ref. Conflicting writes from different clones are
resolved at push/pull time by Git itself — same-ref non-fast-
forward updates are rejected, and the loser must rebase or
merge. For threads with active multi-clone discussion, treat
them like a Git branch: pull before writing.

3.0 has no shared sidecar index, so there is no
"index-out-of-date" failure mode. The optional `refs/forum/index/*`
namespace (rebuildable cache) is regenerated on demand.

## Setup and tools

### Install

```text
cargo install --path .
git forum --version
```

### Repository setup

```text
git forum init
```

`init` creates `./.forum/` with the default `policy.toml`,
configures the `refs/forum/*` refspec for fetch/push, and
prints next-step hints. Run from the repo root.

### Hooks

```text
git forum hook install                   # commit-msg + post-checkout
git forum hook uninstall                 # remove both
```

The `commit-msg` hook validates `Refs:` trailers point at known
threads and rejects commits that reference unknown IDs. The
`post-checkout` hook initializes `git forum` in a new worktree
and repairs the index after `git read-tree` / `git checkout`
operations.

The advanced sub-commands (`hook check-commit-msg`,
`hook fix-index`, `hook worktree-init`) are wired by the hook
scripts themselves; you should not need to run them directly.

## TUI

```text
git forum tui                            # thread list view
git forum tui @1hg98odf                  # open detail view
```

### Display surface (SPEC-3.0)

The TUI shares the snapshot read/write path with the CLI — every
mutation routes through `internal::commands::*::run`. There is no
separate TUI-only writer. Per-screen content:

- **list**: every thread ref, filtered by category/tag/status.
- **detail**: a single thread (header, body, nodes, links,
  evidence). Replies thread under their parent.
- **review**: the open `objection` and `action` set across all
  threads, grouped by thread.
- **diff**: side-by-side `body.md` revisions.

### Colors

Colors follow the terminal theme (`default` for normal, `red` for
blocking guards, `yellow` for warnings, `green` for resolved,
`gray` for retracted/incorporated).

### Controls

| key | action |
|-----|--------|
| `j` / `k` | next / prev row |
| `enter`   | open detail |
| `esc`     | back |
| `c`       | comment |
| `o`       | objection |
| `a`       | action |
| `r`       | resolve |
| `R`       | retract |
| `s`       | state transition |
| `/`       | filter |
| `?`       | help |
| `q`       | quit |

The TUI never auto-saves a draft body — every mutation is a
deliberate keypress that maps to a single `internal::commands::*`
call, and a failed CAS surfaces a "re-read?" prompt rather than
silently overwriting another writer's commit.

## Migration from 1.x / 2.x

```text
git forum migrate --to 3.0               # rewrite every thread ref
git forum migrate --to 3.0 --dry-run     # report what would change
git forum migrate --as ai/migrate        # tag the synthetic actor
```

Migration is one-way and intentionally lossy. The migrator
preserves: thread title, body, readable discussion content,
outgoing links, tags, and the legacy kind/lifecycle mapped to a
3.0 category (SPEC-3.0 §8.1). It does not preserve: 1.x/2.x
state-machine semantics, exact policy outcomes, original
node-type labels (rhetorical shorthands collapse to `comment`
nodes with a `legacy_label` field), or strict event order.

After migration, every `refs/forum/threads/<id>` carries a
3.0 snapshot tree. The original event-chain commits are still
in Git history (the ref's `git log` shows them), but the
authoritative read state is the snapshot at HEAD.

Per ADR-011 Decision 3, only the migrate command may consume
legacy event chains. Any other command that encounters an
unmigrated ref bails with `LegacyEventChain` and asks the
operator to migrate first.

## Per-subcommand help targets

```text
git forum --help                         # top-level
git forum --help-llm                     # this manual
git forum <cmd> --help                   # per-command synopsis
git forum <cmd> --help-llm               # per-command long form
git forum thread new --help              # nested form
```

`--help` prints the clap-rendered short form. `--help-llm` prints
this manual (or the per-command section when invoked under a
subcommand). Both flags work at any depth.
