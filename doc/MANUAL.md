# Manual

This document describes how to use the current `git-forum` CLI.
It is intentionally based on the commands that are implemented today, not future spec ideas.

## Install

```bash
cargo install --path .
git-forum --help
```

If you only want to try it during development, this also works:

```bash
cargo run -- --help
```

## Conventions

- thread kinds: `issue`, `rfc`, `decision`
- thread IDs: `ISSUE-0001`, `RFC-0001`, `DEC-0001`
- node IDs: printed by `git forum say`
- node IDs in CLI arguments:
  - full IDs always work
  - if there is no exact match, a unique prefix of at least 8 characters is accepted
  - `git forum node show` resolves prefixes globally
  - `revise`, `retract`, `resolve`, and `reopen` resolve prefixes inside the specified thread
- actor:
  - if `--as` is omitted, the current actor is inferred from Git config
  - you can override it explicitly, for example `--as human/alice`

## Repository setup

Initialize `git-forum` inside a repository before using it:

```bash
git forum init
git forum doctor
git forum reindex
```

- `init`: creates `.forum/` and `.git/forum/`
- `doctor`: checks policy, templates, local index, and ref namespace health
- `reindex`: rebuilds the local index from Git refs

## Create threads

### Issue

```bash
git forum issue new "Parser fails on nested blocks"
git forum issue new "Parser fails on nested blocks" --body "Repro in src/parser/tests.rs"
git forum issue new "Parser fails on nested blocks" --body-file ./tmp/issue-body.md
```

### RFC

```bash
git forum rfc new "Switch solver backend to trait objects"
git forum rfc new "Switch solver backend to trait objects" \
  --body "Needed to make plugin ABI stability explicit."
```

### Decision

```bash
git forum decision new "Adopt trait backend for v2"
```

### List by kind

```bash
git forum issue ls
git forum rfc ls
git forum decision ls
```

## List and inspect threads

```bash
git forum ls
git forum show RFC-0001
```

`git forum show <THREAD_ID>` shows:

- title
- body
- kind
- status
- created_at
- created_by
- open objections
- open actions
- latest summary
- timeline

The timeline is displayed in `date ID author type body` order.

## Add discussion nodes

### Add a node

```bash
git forum say RFC-0001 --type claim --body "Needed for compatibility."
git forum say RFC-0001 --type question --body "What is the migration plan?"
git forum say RFC-0001 --type objection --body "Benchmarks are missing."
git forum say RFC-0001 --type summary --body "Current consensus is to keep both backends."
```

Valid node types:

- `claim`
- `question`
- `objection`
- `alternative`
- `evidence`
- `summary`
- `decision`
- `action`
- `risk`
- `assumption`

On success, the command prints the node ID.

```text
Added question 0019ccd4b0eb9-928b0ca08d384041
```

### Revise a node

```bash
git forum revise RFC-0001 0019ccd4b0eb9-928b0ca08d384041 \
  --body "What is the migration and rollback plan?"
git forum revise RFC-0001 0019ccd4 \
  --body "What is the migration and rollback plan?"
```

### Retract / resolve / reopen a node

```bash
git forum retract RFC-0001 0019ccd4b0eb9-928b0ca08d384041
git forum resolve RFC-0001 0019ccd4b0eb9-928b0ca08d384041
git forum reopen RFC-0001 0019ccd4b0eb9-928b0ca08d384041
git forum resolve RFC-0001 0019ccd4
```

- `resolve` / `reopen` are mainly for `objection` and `action`
- `retract` keeps history while marking the node inactive

## Inspect a single node

Use this when you want to inspect one node directly instead of reading the whole thread:

```bash
git forum node show 0019ccd4b0eb9-928b0ca08d384041
git forum node show 0019ccd4
```

`git forum node show <NODE_ID>` shows:

- node ID
- the thread it belongs to
- kind
- current state: `open`, `resolved`, `retracted`
- created_at
- actor
- current body
- the history related to that node

If a prefix is ambiguous, the command fails and prints candidate full IDs.

## Change thread state

```bash
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum state RFC-0001 accepted --sign human/alice
git forum state RFC-0001 accepted --sign human/alice --sign human/bob
```

- `--sign` is recorded as an approval on the event
- whether the transition succeeds depends on the state machine and policy guards

## Verify and inspect policy

```bash
git forum verify RFC-0001
git forum policy lint
git forum policy check RFC-0001 --transition under-review->accepted
```

- `verify`: checks whether the thread already satisfies guard conditions for its next forward transition
- `policy lint`: validates `.forum/policy.toml`
- `policy check`: dry-runs guard evaluation for a specific transition

### The policy file

The policy file lives at `.forum/policy.toml`.

It is created automatically by `git forum init`, and it controls two kinds of configuration:

- role declarations under `[roles.<name>]`
- transition guard rules under `[[guards]]`

A default file looks like this:

```toml
[roles.reviewer]
can_say = ["objection", "evidence", "summary", "risk"]
can_transition = ["under-review->changes-requested"]

[roles.maintainer]
can_say = ["claim", "decision", "summary"]
can_transition = ["draft->proposed", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]
```

### What the fields mean

- `can_say`: node types that a role is allowed to emit
- `can_transition`: thread state transitions that a role is allowed to perform
- `on`: the transition that a guard block applies to, written as `from->to`
- `requires`: the list of guard rules that must pass for that transition

### Guard rules currently understood by the implementation

- `no_open_objections`
- `at_least_one_summary`
- `one_human_approval`

### What is enforced today

Current implementation status is narrower than the long-term spec:

- `git forum state ...` evaluates guard rules from `[[guards]]`
- `git forum verify` evaluates those same guard rules in read-only mode
- `git forum policy lint` currently performs structural validation, mainly checking that guard transitions use the `from->to` format

Role sections such as `can_say` and `can_transition` are parsed and preserved, but they are not yet fully enforced across all commands.

### Editing the policy file

The file is plain TOML, so the normal workflow is:

1. edit `.forum/policy.toml`
2. run `git forum policy lint`
3. run `git forum policy check ...` or `git forum verify ...`

Example:

```bash
git forum policy lint
git forum policy check RFC-0001 --transition under-review->accepted
git forum verify RFC-0001
```

### What `git forum verify` actually does

`git forum verify` is read-only. It does not change the thread state and it does not attach approvals.

At the moment, it only evaluates a small set of forward transitions:

- RFC in `under-review` is checked against `under-review -> accepted`
- Decision in `proposed` is checked against `proposed -> accepted`
- other kinds or states currently return `ok` because no verify target is defined

In practice, this means `verify` is most useful right before an acceptance-like transition. It answers: "If I tried to move this thread forward now, which guard checks would fail?"

### Typical output

If all configured guards pass:

```text
RFC-0001: ok
```

If one or more guards fail:

```text
FAIL [no_open_objections] unresolved objections remain
FAIL [at_least_one_summary] no summary node found
```

### `verify` vs `policy check`

Use `verify` when you want the tool to infer the next forward transition from the current thread state.

Use `policy check` when you want to ask about a specific transition explicitly:

```bash
git forum policy check RFC-0001 --transition under-review->accepted
git forum policy check DEC-0001 --transition proposed->accepted
```

## Typical workflow

```bash
git forum init
git forum rfc new "Switch solver backend to trait objects" \
  --body "Needed to make plugin ABI stability explicit."
git forum say RFC-0001 --type claim --body "Needed for compatibility."
git forum say RFC-0001 --type question --body "What is the migration plan?"
git forum show RFC-0001
git forum node show 0019ccd4b0eb9-928b0ca08d384041
git forum state RFC-0001 proposed
git forum state RFC-0001 under-review
git forum verify RFC-0001
```

## Current scope

This manual currently covers:

- init / doctor / reindex
- issue / rfc / decision create and list
- thread show
- node show
- say / revise / retract / resolve / reopen
- state
- verify
- policy lint / check

Still out of scope:

- evidence commands
- AI run commands
- import / export
- TUI
