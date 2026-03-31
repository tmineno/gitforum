# Manual

## Quick Reference

```
# create
git forum new <kind> "Title" [--body "..."|--edit] Create a thread

# inspect
git forum ls [--kind <kind>]                       List threads
git forum show <ID>                                Show thread details
git forum show <ID> --what-next                    Show valid next actions
git forum show <ID> --compact                      Compact single-line view
git forum show <ID> --no-timeline                  Omit timeline from output
git forum log <ID>                                 Show event timeline for a thread
git forum log <ID> --reverse                       Show newest events first
git forum log <ID> -n <N>                          Limit to last N events
git forum search <query>                           Search threads and nodes
git forum shortlog --since <DATE_OR_REV>           Threads resolved after date/tag
git forum status <ID>                              Check open items
git forum node show <NODE_ID>                      Inspect a single node

# discussion (canonical + shorthands)
git forum node add <ID> --type <type> "body"       Add a typed node
git forum claim <ID> "body"                        node add --type claim
git forum question <ID> "body"                     node add --type question
git forum objection <ID> "body"                    node add --type objection
git forum summary <ID> "body"                      node add --type summary
git forum action <ID> "body"                       node add --type action
git forum risk <ID> "body"                         node add --type risk
git forum review <ID> "body"                       node add --type review
git forum retype <ID> <NODE_ID> --type <TYPE>      Change a node's type
git forum resolve <ID> <NODE_ID>                   Resolve a node
git forum retract <ID> <NODE_ID>                   Retract a node
git forum reopen <ID> <NODE_ID>                    Reopen a node

# state (canonical + shorthands)
git forum state <ID> <state>                       Change thread state
git forum state <ID> <state> --approve human/alice State change with approval
git forum state <ID> <state> --comment "Done"      State change with comment
git forum state bulk --to <state> [--kind <kind>]  Bulk state change
git forum close <ID>                               state <ID> closed
git forum pend <ID>                                state <ID> pending
git forum accept <ID> --approve human/alice        state <ID> accepted
git forum propose <ID>                             state <ID> proposed
git forum reject <ID>                              state <ID> rejected
git forum deprecate <ID>                           state <ID> deprecated

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
git forum hook install                             Install commit-msg hook
git forum tui                                      Open interactive TUI
git forum purge --thread <ID> --event <SHA>        Purge event content
git forum purge --actor <ACTOR_ID>                 Purge all events by actor
```

## Conventions

- thread kinds: `ask` (alias: `issue`), `rfc`, `dec`, `job` (alias: `task`)
- thread IDs: opaque `KIND-XXXXXXXX` (e.g. `RFC-a7f3b2x1`) for new threads; legacy sequential `KIND-NNNN` (e.g. `ASK-0001`) also accepted. Unambiguous prefixes work (e.g. `RFC-a7f3`).
- node IDs: printed by shorthand node commands (e.g. `claim`, `question`); canonical IDs are Git commit OIDs of the say event
- CLI/TUI displays of node and event OIDs usually show the first 16 characters
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

## Thread kinds

Before creating a thread, select the appropriate kind:

| If the work...                                    | Use   |
|---------------------------------------------------|-------|
| Affects multiple teams, hard to reverse            | rfc   |
| Is a local design decision worth recording         | dec   |
| Is an implementable unit of work with clear scope  | job   |
| Is a bug report or feature request                 | ask   |

Rules of thumb:
- If you are comparing alternatives → dec
- If you are defining acceptance criteria → job
- If you need cross-team sign-off → rfc
- If something is broken or missing → ask
- When in doubt between dec and rfc, start with dec — it can be escalated to an rfc later
- When in doubt between job and ask, prefer job if you know the implementation path

DEC threads should include at least one `alternative` node documenting what was not chosen.
JOB threads should include `assumption` nodes for any dependencies the work relies on.

Use `git forum node add <ID> --type alternative "..."` and `--type assumption "..."` to create these nodes.

Agents are participants, not a separate control plane. For cross-cutting alignment, start with an RFC. For team-internal design reasoning, use a DEC. Break implementation into JOBs. Track bugs and requests as ASKs.

## Create threads

### RFC

Use RFCs to frame work before implementation starts.

```bash
git forum new rfc "Switch solver backend to trait objects"
git forum new rfc "Switch solver backend to trait objects" \
  --body "Goal, constraints, and acceptance."
git forum new rfc "Switch solver backend to trait objects" --body -
git forum new rfc "Switch solver backend to trait objects" --body-file ./tmp/rfc.md
git forum new rfc "Switch solver backend to trait objects" --edit
```

### DEC (Decision Record)

Use DECs to record local design decisions worth preserving. Requires a body.

```bash
git forum new dec "Use Redis over Memcached" --body-file ./tmp/dec.md
git forum node add DEC-0001 --type alternative "Memcached: simpler, but no pub/sub"
git forum node add DEC-0001 --type assumption "Redis cluster available in prod"
git forum state DEC-0001 accepted
```

DEC lifecycle: `proposed` → `accepted` / `rejected` → `deprecated`

### JOB

Use JOBs for implementable work units with clear scope.

> **Alias:** `task` still works as an alias for `job` in all commands.

```bash
git forum new job "Implement Redis cache client wrapper"
git forum state JOB-0001 designing
git forum state JOB-0001 implementing
git forum state JOB-0001 reviewing
git forum state JOB-0001 closed

# Fast-track for trivial jobs:
git forum new job "Add cache-control headers"
git forum state JOB-0002 closed
```

JOB lifecycle: `open` → `designing` → `implementing` → `reviewing` → `closed`
Back-transitions: `implementing` → `designing`, `reviewing` → `implementing`
Fast-track: `open` → `closed` (for trivial jobs)

### Ask

Use asks for implementation work, especially when code or a branch is involved.

> **Alias:** `issue` still works as an alias for `ask` in all commands.

```bash
git forum new ask "Implement trait backend"
git forum new ask "Implement trait backend" --body "Initial implementation checklist"
git forum new ask "Implement trait backend" --body -
git forum new ask "Implement trait backend" --body-file ./tmp/issue.md
git forum new ask "Implement trait backend" --edit
git forum new ask "Implement trait backend" --branch feat/trait-backend
git forum new ask "Implement trait backend" \
  --link-to RFC-0001 --rel implements
git forum new rfc "Error handling" \
  --claim "All errors should be typed" --action "Define error enum"
```

`--body -` reads the initial body from standard input, so you can avoid creating a temporary file.
`--edit` opens `$VISUAL` / `$EDITOR` / `vi` for interactive body composition. Lines starting with
`#` are stripped. Empty content aborts the command. `--edit` conflicts with `--body` and
`--body-file`. `--edit` requires an interactive terminal; in scripts or agent workflows, use
`--body`, `--body-file`, or `--body -` instead.

### Inline nodes at creation

Thread creation accepts `--claim`, `--question`, `--objection`, `--action`, `--risk`, and
`--summary` flags to add nodes immediately after creating the thread. Each flag may be repeated:

```bash
git forum new rfc "Caching layer" --body "Goal and constraints." \
  --claim "LRU eviction with 10-min TTL" \
  --action "Benchmark cache hit ratio" \
  --risk "Memory pressure under load"
```

This is equivalent to running `new rfc` followed by separate `claim`, `action`, and `risk` commands.
`--branch <BRANCH>` binds the new thread to an existing Git branch.
`--link-to <THREAD_ID> --rel <REL>` creates the thread and immediately records one or more links
from the new thread to existing threads.

### Create from a commit

```bash
git forum new ask --from-commit HEAD
git forum new ask --from-commit abc123 --link-to RFC-0001 --rel implements
```

`--from-commit <REV>` uses the commit subject as the title, the commit body as the thread body,
and automatically adds the commit as evidence. An explicit title argument overrides the subject.

### Create from another thread

```bash
git forum new rfc --from-thread RFC-0003
git forum new ask --from-thread ASK-0001
git forum new rfc --from-thread ASK-0005 "Custom title"
```

`--from-thread <THREAD_ID>` copies the title (prefixed with `v2: `) and body from the source
thread and creates bidirectional `supersedes` / `superseded-by` links. An explicit title argument
overrides the default title.

Behavior depends on the source and target kinds:

- **RFC → new RFC**: source RFC is auto-deprecated (supersession).
- **Ask → new ask**: source ask is unchanged (respin/split).
- **Ask → new RFC**: source ask is unchanged (elevation to formal proposal).
- **RFC → new ask**: not allowed — use `git forum link --rel implements` instead.

### List by kind

```bash
git forum ls --kind ask
git forum ls --kind ask --branch feat/trait-backend
git forum ls --kind rfc
git forum ls dec                                   # positional shorthand
git forum ls job
```

The old forms `git forum issue ls`, `git forum rfc ls`, etc. remain as hidden aliases for backward
compatibility. `--kind issue` and `--kind task` also still work.

## Structured discussion

### Add a node

Each node type has a dedicated shorthand command.
All node commands accept a positional body argument, `--body-file`, `--edit`, and `--as`. Pass `"-"` as the
positional body to read from stdin. Use `--edit` to compose in `$EDITOR`.

```bash
git forum claim RFC-0001 "Need a stable plugin-facing boundary."
git forum question RFC-0001 "What compatibility risks remain?"
git forum objection RFC-0001 "Benchmarks are missing."
git forum summary RFC-0001 "Direction is sound, but migration evidence is missing."
git forum action ASK-0001 "Add branch-local benchmark fixture."
git forum risk ASK-0001 "Parser behavior may diverge under edge inputs."
git forum review RFC-0001 "Overall analysis of the RFC."
git forum objection RFC-0001 --body-file ./tmp/detailed-objection.md
git forum claim RFC-0001 --body -
git forum review RFC-0001 --edit
```

Valid node types:

- `claim`, `question`, `objection`, `evidence`, `summary`, `action`, `risk`, `review` — shorthand commands available
- `alternative`, `assumption` — use `git forum node add` instead:

```bash
git forum node add DEC-0001 --type alternative "Use Memcached instead"
git forum node add JOB-0001 --type assumption "Redis cluster is available"
```

`review` is a holistic analysis of the entire thread, distinct from `claim` (single assertion) and
`summary` (consensus digest). Reviews are informational and typically not resolvable.

`summary` is not just another comment. In the default workflow it is the human-readable statement of
what the thread currently concludes, what objections were addressed, and what is ready to move
forward. The default policy therefore requires at least one `summary` before an RFC can move to
`accepted`.

On success, the command prints the node ID to stdout and a next-actions hint to stderr:

```text
Added question 6f1d2c3b4a5e67890123456789abcdef01234567
  next: proposed, rejected
  open: 1 open objection(s)
```

The hint shows valid state transitions and open items. Suppress with `2>/dev/null`.

### Retype a node

Use `retype` to change the type of an existing node:

```bash
git forum retype RFC-0001 6f1d2c3b --type claim
```

The old type is recorded in the event for auditability. Accepts `--as` and `--force`.

### Revise a node

```bash
git forum revise node RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567 \
  --body "What is the migration and rollback plan?"
git forum revise node RFC-0001 6f1d2c3b \
  --body "What is the migration and rollback plan?"
git forum revise node RFC-0001 6f1d2c3b --edit
```

Use `revise node` to update an existing node when the intent is the same but the content needs
correction. For example, revise a summary to incorporate new objections rather than adding a
second summary node. The revision history is preserved and visible in `git forum node show`.

### Retract / resolve / reopen a node

All three commands accept one or more node IDs:

```bash
git forum retract RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum reopen RFC-0001 6f1d2c3b4a5e67890123456789abcdef01234567
git forum resolve RFC-0001 6f1d2c3b
git forum retract RFC-0001 node1 node2 node3    # retract multiple nodes
git forum resolve RFC-0001 node1 node2          # resolve multiple nodes
```

- `resolve` / `reopen` are mainly for `objection` and `action`
- `retract` is a **soft-delete**: it marks the node inactive but the original body text remains
  in git history. Anyone with repo access can read retracted content via `git log`. Do not use
  retract for removing sensitive data — there is currently no hard-delete mechanism.
- when multiple node IDs are given, each node is processed independently; failures are reported
  inline on stderr and the command exits non-zero if any fail

### Reply to a node

Use `--reply-to` to link a node as a response to an existing node:

```bash
git forum claim RFC-0001 "Tests added, benchmark in bench/result.csv" \
  --reply-to <OBJECTION_NODE_ID>
git forum question RFC-0001 "Can you clarify X?" --reply-to <CLAIM_NODE_ID>
```

`--reply-to` is accepted on all shorthand node commands. Reply chains of arbitrary depth are
supported. `git forum show` groups reply chains into conversations for readability.

### Inspect a single node

Use this when you want to inspect one node directly instead of reading the whole thread:

```bash
git forum node show 6f1d2c3b4a5e67890123456789abcdef01234567
git forum node show 6f1d2c3b
```

`git forum node show <NODE_ID>` shows:

- node ID
- the thread it belongs to
- kind
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
git forum status RFC-0001
```

`git forum status <THREAD_ID>` shows unresolved items grouped by type: open objections, open
actions, and open questions.

### Shorthand commands

State shorthands are top-level convenience aliases (verb-first):

```bash
git forum close ASK-0001
git forum close ASK-0001 --comment "Fixed in abc123"
git forum close ASK-0001 --link-to RFC-0001 --rel implements
git forum close ASK-0001 --resolve-open-actions
git forum pend ASK-0001                              # mark as pending
git forum pend ASK-0001 --comment "Waiting on review"
git forum state ASK-0001 open                       # thread state reopen
git forum reject ASK-0001 --comment "Won't fix"
git forum propose RFC-0001
git forum accept RFC-0001 --approve human/alice
git forum deprecate RFC-0001 --comment "Superseded by RFC-0005"
git forum state RFC-0001 deprecated --link-to RFC-0005 --rel relates-to
```

Shorthand commands combine a state transition with optional `--comment` (attaches comment text to
the state-change event's body), `--link-to` (creates links after transitioning), and `--approve`
(records approvals).

### Generic state command

```bash
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum state RFC-0001 accepted --approve human/alice
git forum state ASK-0001 closed --resolve-open-actions
git forum state ASK-0001 closed --comment "Done" --link-to RFC-0001 --rel implements
git forum state bulk --to closed --branch v0.1.0
git forum state bulk --to closed ASK-0001 ASK-0002 --dry-run
```

- `--approve` is recorded as an approval on the event
- recorded approvals are not cryptographically verified in the MVP
- `--comment` attaches comment text to the state-change event's body (visible in the timeline)
- `--link-to` and `--rel` create thread links after the state transition
- whether the transition succeeds depends on the state machine and policy guards
- for RFCs, `proposed` means the author is declaring the RFC review-ready
- for RFCs, `under-review` means active review is in progress
- an accepted RFC is the decision record; there is no separate decision workflow in the preferred model
- asks support `open`, `pending`, `closed`, and `rejected` states; `pending` is for
  work-in-progress or waiting, `rejected` is for invalid or won't-fix asks, `closed` means
  completed
- DECs support `proposed`, `accepted`, `rejected`, and `deprecated` states
- JOBs support `open`, `designing`, `implementing`, `reviewing`, `closed`, and `rejected` states;
  use `git forum state JOB-0001 designing` for phase transitions
- if policy requires `no_open_actions`, closing an ask or job with open `action` nodes fails
- `--resolve-open-actions` is an explicit escape hatch for ask/job close; it resolves open `action`
  nodes before writing the closing state event
- `state bulk` evaluates each target independently, applies successful transitions, reports
  failures inline, and exits non-zero if any target failed
- `state bulk --dry-run` reports what would succeed or fail without writing any events

## Search, list, show

### List threads

```bash
git forum ls
git forum ls --branch feat/trait-backend
git forum show RFC-0001
git forum show RFC-0001 --what-next
```

`git forum ls` shows `ID`, `KIND`, `STATUS`, `BRANCH`, `CREATED`, `UPDATED`, and `TITLE`.
`--kind rfc`, `--kind ask`, `--kind dec`, or `--kind job` filters by thread kind
(`issue` and `task` still work as aliases).
`--branch <BRANCH>` filters the listing to threads currently bound to that branch.

### Show thread details

`git forum show <THREAD_ID>` shows:

- title, kind, status
- **next**: compact list of valid transitions with guard status (e.g. `accepted (blocked: no_open_objections), rejected, draft`)
- **transitions**: Unicode state diagram with current state highlighted in brackets
- created_at, created_by, branch
- body
- body revisions count (if body has been revised)
- incorporated nodes (if any)
- open objections, open actions
- latest summary
- evidence, links
- conversations (reply chains grouped by root node)
- timeline

`git forum show <THREAD_ID> --what-next` shows valid next actions plus operation check
rules for the current state:

```text
RFC-0001 (under-review)

valid transitions: accepted, rejected, draft

guard check (under-review -> accepted):
  [FAIL] no_open_objections -- 1 open objection(s)

open objections: 1
open actions:    0
nodes:           6
evidence:        1
links:           0
has summary:     yes

operation checks (state: under-review):
  node types: (all allowed)
  body revise: allowed
  evidence:    allowed
```

Three discoverability surfaces exist:

- **`show`**: compact, thread-specific — `next:` line and state diagram
- **`show --what-next`**: detailed, thread-specific — guard checks plus operation check rules
- **`policy show`**: global — full policy as loaded from `.forum/policy.toml`

The timeline is displayed in `date node_id event_id author type body` order.

If the thread has evidence or links attached, they appear between the summary and the timeline:

```text
evidence: 1
  - a1b2c3d4  benchmark  bench/result.csv

links: 1
  - ASK-0001  implements
```

### Log

`git forum log <THREAD_ID>` shows the event timeline for a thread as a standalone command.

```bash
git forum log RFC-0001
git forum log RFC-0001 --reverse    # newest events first
git forum log RFC-0001 -n 5         # last 5 events
```

This is the timeline from `git forum show` as a standalone view. `--reverse` shows newest events first. `-n N` limits to the last N events.

### Search

```bash
git forum search migration
git forum search objection
git forum search RFC-0001
```

`git forum search <QUERY>` searches the local index across:

- thread title
- thread body
- thread kind and state
- thread ID
- current node body
- current node type and node ID

Results are grouped by thread. If the match came from a current node, the output includes the
matching node under the thread row.

## Evidence and links

### Add evidence to a thread

```bash
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum evidence add ASK-0001 --kind commit --ref HEAD~1
git forum evidence add ASK-0001 --kind commit --ref abc123def456
git forum evidence add ASK-0001 --kind commit --ref abc123 def456 789012
git forum evidence add ASK-0001 --kind file --ref src/lib.rs
git forum evidence add ASK-0001 --kind test --ref tests/backend_trait.rs
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

After implementing work on a branch, link the commits back to the RFC or ask so that the
decision trail connects to the code:

```bash
git forum evidence add ASK-0001 --kind commit --ref HEAD
git forum evidence add ASK-0001 --kind test --ref tests/cache_test.rs
git forum evidence add ASK-0001 --kind thread --ref RFC-0001
```

`--kind commit --ref` accepts any Git revision expression (SHA, branch, tag, `HEAD~1`). The
resolved commit OID is stored canonically.

### Link two threads

```bash
git forum link ASK-0001 RFC-0001 --rel implements
git forum link ASK-0002 ASK-0001 --rel depends-on
git forum link ASK-0003 ASK-0002 --rel blocks
git forum link RFC-0002 RFC-0001 --rel relates-to
```

On success:

```text
ASK-0001 -> RFC-0001 (implements)
```

`--rel` is currently free-form. Common values are `implements`, `relates-to`, `depends-on`, and
`blocks`.

### Bind a thread to a Git branch

```bash
git forum branch bind ASK-0001 feat/parser-rewrite
git forum branch clear ASK-0001
```

This updates the thread's `scope.branch`. It is most useful for issues that track implementation
work on a feature branch, but the command is available for any thread kind.

## Body revision and diff

### Revise thread body

`body` is the default target for `revise` — the `body` keyword is optional:

```bash
git forum revise RFC-0001 --body "Updated body text"
git forum revise RFC-0001 --body-file ./tmp/body.md
git forum revise RFC-0001 --body -
git forum revise RFC-0001 --edit
git forum revise body RFC-0001 --body "Updated body text"   # explicit, still works
```

`--incorporates` marks referenced nodes as incorporated into this revision:

```bash
git forum revise RFC-0001 --body "Revised body" \
  --incorporates 6f1d2c3b --incorporates a1b2c3d4
```

Incorporated nodes appear as `incorporated` status in show output, distinct from `resolved` and
`retracted`. They represent content that has been folded into the current body.

### Diff body revisions

After revising a thread body, use `diff` to see what changed between revisions:

```bash
git forum diff RFC-0001                            # latest vs previous revision
git forum diff RFC-0001 --rev 1                    # diff revision 0 vs 1
git forum diff RFC-0001 --rev 0..2                 # diff revision 0 vs 2
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
git forum verify RFC-0001
git forum policy show
git forum policy lint
git forum policy check RFC-0001 --transition under-review->accepted
```

- `verify`: preflight check — tests whether the thread is ready for its next forward transition (not a history audit)
- `policy show`: displays the loaded policy in human-readable format (guards, creation rules, operation checks, strict mode). Only shows configured sections — no synthesized defaults
- `policy lint`: validates `.forum/policy.toml` — checks guard syntax, unknown states, invalid transitions, and warns when allow-lists miss entire thread kinds
- `policy check`: dry-runs guard evaluation for a specific transition

### The policy file

The policy file lives at `.forum/policy.toml`.

It is created automatically by `git forum init`, and it controls:

- **Transition guards** (`[[guards]]`): rules that must pass for a state transition.
- **Operation checks** (`creation_rules`, `node_rules`, `revise_rules`, `evidence_rules`, `checks`): rules that validate write operations before committing events.

A full example:

```toml
[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]

[[guards]]
on = "proposed->accepted"
requires = ["no_open_objections"]

[[guards]]
on = "reviewing->closed"
requires = ["no_open_actions"]

[checks]
strict = false

[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[creation_rules.ask]
required_body = false
body_sections = []

[creation_rules.dec]
required_body = true
body_sections = ["Context", "Decision", "Rationale", "Impact"]

[creation_rules.job]
required_body = false
body_sections = ["Background", "Acceptance criteria", "Exceptions"]

[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending", "designing", "implementing"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing"]

[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending", "designing", "implementing", "reviewing", "closed", "accepted", "rejected", "deprecated"]
```

### Guard rules

#### Guard fields

- `on`: the transition that a guard block applies to, written as `from->to`
- `requires`: the list of guard rules that must pass for that transition

#### Operation check fields

- `[checks]`: global check settings
  - `strict`: when `true`, warnings become errors (unless `--force` is used). Default: `false`.
- `[creation_rules.<kind>]`: rules for creating threads of a given kind (e.g., `rfc`, `ask`)
  - `required_body`: if `true`, the thread must have a non-empty body (Error if missing)
  - `body_sections`: list of section headings to check for in the body (Warning if missing)
- `[node_rules]`: maps state names to lists of allowed node types in that state (Error if violated). An absent state means all node types are allowed.
- `[revise_rules]`: controls in which states revision is allowed
  - `allow_body_revise`: list of states where body revision is allowed (Error if violated)
  - `allow_node_revise`: list of states where node revision is allowed (Error if violated)
- `[evidence_rules]`: controls in which states evidence can be attached
  - `allow_evidence`: list of states where evidence attachment is allowed (Error if violated)

### Guard rules currently understood by the implementation

- `no_open_objections`
- `no_open_actions`
- `at_least_one_summary`
- `one_human_approval`
- `has_commit_evidence`

### Operation checks

Operation checks validate write commands against policy rules before committing events. They are
evaluated at the CLI boundary on `new`, node commands, `revise`, and `evidence add`.

| Policy section | Commands checked | What it validates |
|----------------|------------------|-------------------|
| `[creation_rules.<kind>]` | `new` | Required body, required body sections (headings) |
| `[node_rules]` | `claim`, `question`, etc. | Node type allowed in the current thread state |
| `[revise_rules]` | `revise` | Revision allowed in the current thread state |
| `[evidence_rules]` | `evidence add` | Evidence addition allowed in the current thread state |

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

- No policy file → all checks pass (no restrictions)
- Missing policy sections → no restrictions for that check (`#[serde(default)]`)

### What is enforced today

- `git forum state ...` evaluates guard rules from `[[guards]]`
- `git forum verify` is a read-only preflight that evaluates those same guard rules without changing state
- `git forum show` displays compact next-states with guard blockers and a state diagram
- `git forum show --what-next` displays detailed guard checks and operation check rules for the current state
- `git forum policy show` displays the loaded policy in human-readable format
- `git forum policy lint` validates guard transitions and detects semantic gaps in operation allow-lists
- All write commands evaluate operation checks from `[creation_rules]`, `[node_rules]`,
  `[revise_rules]`, and `[evidence_rules]`


### What `git forum verify` actually does

`git forum verify` is a **preflight check**, not a history audit or integrity verifier. It is read-only — it does not change thread state or attach approvals.

It evaluates policy guards for the thread's next forward transition:

- Ask in `open` → checks guards for `open->closed`
- RFC in `under-review` → checks guards for `under-review->accepted`
- DEC in `proposed` → checks guards for `proposed->accepted`
- JOB in `reviewing` → checks guards for `reviewing->closed`
- Other states → reports `ready` (no preflight target defined)

Use it right before an acceptance-like transition. It answers:
"If I tried to advance this thread now, which guards would block?"

### Which diagnostic command should I use?

| I want to...                                  | Command                  | Scope      |
|-----------------------------------------------|--------------------------|------------|
| See what's blocking a thread                  | `show --what-next`       | thread     |
| Check if a thread is ready to advance         | `verify`                 | thread     |
| Test guards for a specific transition         | `policy check`           | thread     |
| List unresolved objections/actions/questions   | `status`                 | thread     |
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
git forum claim RFC-0001 "Needed for compatibility."
git forum question RFC-0001 "What is the migration plan?" --as ai/reviewer
git forum objection RFC-0001 "Benchmarks are missing."
git forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
git forum summary RFC-0001 "Benchmarks added; objection addressed."
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum propose RFC-0001
git forum state RFC-0001 under-review
git forum verify RFC-0001
git forum accept RFC-0001 --approve human/alice
git forum new ask "Implement trait backend" --link-to RFC-0001 --rel implements
git forum branch bind ASK-0001 feat/trait-backend
git forum action ASK-0001 "Wire trait backend behind feature flag."
git forum evidence add ASK-0001 --kind test --ref tests/backend_trait.rs
git forum close ASK-0001
```

### AI-agent workflow

The same CLI surface works for AI agents. The typical pattern is:

1. A human or agent opens an RFC and adds initial claims.
2. An AI reviewer posts objections and questions using `--as ai/reviewer`.
3. An implementer (human or agent) replies to each objection with evidence or claims.
4. A human resolves addressed objections, adds a summary, and signs the acceptance.

```bash
# 1. Human opens the RFC
git forum new rfc "Add caching layer" --body "Goal and constraints."

# 2. AI reviewer raises concerns
GIT_FORUM_ACTOR=ai/reviewer
git forum objection RFC-0001 "No eviction strategy described."
git forum question RFC-0001 "What is the expected cache hit ratio?"

# 3. Implementer responds to the objection
git forum claim RFC-0001 "LRU eviction with 10-minute TTL." \
  --reply-to <OBJECTION_NODE_ID>

# 4. Human resolves the objection, summarizes, and accepts
git forum resolve RFC-0001 <OBJECTION_NODE_ID>
git forum summary RFC-0001 "Caching with LRU eviction approved."
git forum propose RFC-0001
git forum state RFC-0001 under-review
git forum accept RFC-0001 --approve human/alice
```

**Non-interactive body input for agents:** Since agents run without a TTY, `--edit` will
not work. Use `--body "..."` for short text, `--body-file <path>` for longer content, or
pipe through stdin with `--body -`:

```bash
echo "Detailed review body..." | git forum review RFC-0001 --body -
cat /tmp/review.md | git forum revise RFC-0001 --body -
```

## Concurrency

`git-forum` uses Git's atomic ref updates (compare-and-swap) to detect concurrent writes. Each
`write_event` call reads the current thread ref tip, creates a new commit, and atomically updates
the ref only if the tip has not changed since it was read.

If two writers attempt to update the same thread simultaneously, one will succeed and the other will
fail with a clear error:

```text
concurrent write conflict on refs/forum/threads/RFC-0001: expected <sha> but ref was updated by another writer. Retry your command.
```

**Recommended patterns for parallel agent workflows:**

- **Different threads**: fully safe in parallel. Each thread has its own ref.
- **Same thread**: serialize writes, or retry on conflict. Conflicts are rare for human workflows
  but more likely when multiple agents update the same thread simultaneously.
- **Create vs update**: thread creation uses `create_ref` which fails if the ref already exists,
  preventing duplicate thread IDs.

This is related to ISSUE-0006 (semantic merge for concurrent events), which would automatically
resolve non-conflicting concurrent writes to the same thread.

## Per-subcommand help targets

For focused reference on a specific topic, use per-subcommand `--help-llm`:

```
git forum claim --help-llm       Node type taxonomy (10 types)
git forum state --help-llm       State transition map (all kinds)
git forum evidence --help-llm    Evidence kinds reference (8 kinds)
```

Per-subcommand `--help-llm` prints a focused ~200-token reference section instead of the full
manual. All node shorthand commands (`claim`, `question`, `objection`, `summary`, `action`,
`risk`, `review`, `alternative`, `assumption`, `node`) print the node type taxonomy. All state
commands (`state`, `close`, `pend`, `accept`, `propose`, `reject`, `deprecate`) print the state
transition map. `evidence` prints the evidence kinds reference. All other subcommands print
this full manual.

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

- `init`: creates `.forum/` and `.git/forum/`, installs the commit-msg hook
- `doctor`: checks `.forum/` and `.git/forum/` directories exist, validates `policy.toml` syntax, verifies template files are present and non-empty, checks SQLite index health (integrity and freshness), and replays every thread's event log to verify integrity. Reports `[ok]`, `[WARN]`, or `[FAIL]` per check; exits non-zero only on failures (warnings are informational)
- `reindex`: rebuilds the local index from Git refs

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

#### Commit-msg hook

`git forum init` automatically installs an advisory `commit-msg` hook that validates thread ID
references in commit messages. The hook can also be managed manually:

```bash
git forum hook install              # install the commit-msg hook
git forum hook install --force      # overwrite an existing hook (no backup)
git forum hook uninstall            # remove the git-forum hook
```

The hook delegates to `git-forum hook check-commit-msg <file>`, which:

1. Strips Git comment lines (respecting `core.commentChar`) and scissors sections.
2. Scans the cleaned message for thread ID patterns: both legacy sequential (`ASK-NNNN`, `RFC-NNNN`, `DEC-NNNN`, `JOB-NNNN`, `ISSUE-NNNN`, `TASK-NNNN`) and opaque content-addressed (`KIND-XXXXXXXX` where X is base36).
3. Validates each referenced thread exists in `refs/forum/threads/`.

**Behavior:**

- No thread IDs found: prints a warning, exits 0 (commit proceeds).
- All referenced threads exist: exits 0 silently.
- Any referenced thread missing: prints a warning with the missing IDs, exits 1 (commit blocked).

```text
git-forum: commit message references non-existent thread(s):
  ASK-9999 — not found
hint: create the thread first, or remove the reference from the commit message.
```

The hook path is resolved via `git rev-parse --git-path hooks/commit-msg`, so it works correctly
with worktrees and `core.hooksPath`. `--force` overwrites any existing hook without backup; users
with custom hooks should use a hook dispatcher (e.g., the pre-commit framework).

### Purge (hard-delete)

`git forum purge` permanently removes event content from git history by rewriting commits.
This is destructive: commit SHAs change and all clones must re-fetch affected refs.

#### Purge a specific event

```bash
git forum purge --thread ASK-0001 --event <SHA>
git forum purge --thread ASK-0001 --event <SHA> --dry-run
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
git forum tui RFC-0001
```

### Colors

The TUI uses color to distinguish kinds, statuses, and node types:

- **Thread kind**: cyan = rfc, yellow = ask, magenta = dec, green = job
- **Thread status**: green = open/draft, yellow = pending/proposed/under-review/designing/implementing/reviewing,
  magenta = accepted/closed, red = rejected, gray = deprecated
- **Node type**: red = objection/risk, yellow = question, green = summary, cyan = action,
  blue = review
- **Node status**: green = open, gray = resolved/retracted/incorporated

Resolved, retracted, and incorporated node rows are dimmed.

### Controls

- **List view**: `j`/`k` navigate, `enter` opens thread, `c` creates, `f` cycles kind filter, `r` refreshes, `q` quits. Click column headers to sort; click/double-click rows to select/open.
- **Thread detail**: `j`/`k` navigate nodes, `up`/`down` scroll body, `enter` opens node, `c` creates node, `l` creates link, `m` toggles markdown, `S` enters select mode, `r` refreshes, `esc`/`q` goes back.
- **Node detail**: `c` creates node, `l` creates link, `x` resolves, `o` reopens, `R` retracts, `m` toggles markdown, `j`/`k` scroll, `r` refreshes, `esc`/`q` goes back.
- **Create forms**: `tab` moves between fields, `up`/`down` cycles kind/type, `enter` on body opens editor, `enter` on submit creates, `ctrl+s` in body editor returns to form, `esc` cancels.
