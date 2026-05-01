use super::event::Lifecycle;
use super::state_machine;

/// Node type taxonomy for --help-llm.
pub fn node_type_taxonomy() -> String {
    r#"# Node Type Taxonomy

| Type | Purpose | Resolvable |
|------|---------|------------|
| claim | Assert a fact or position | no |
| question | Ask for clarification or information | no |
| objection | Raise a blocking concern (must be resolved before acceptance) | yes |
| evidence | Reference supporting data (distinct from evidence attachment) | no |
| summary | Consensus digest — what the thread concludes | no |
| action | A task to be completed (must be resolved before closing) | yes |
| risk | Flag a potential problem or uncertainty | no |
| review | Holistic analysis of the entire thread | no |
| alternative | Record a considered alternative approach | no |
| assumption | Record an assumption the design depends on | no |

## When to use each

- **claim**: single assertions ("We should use trait objects")
- **question**: requests for info ("What is the migration plan?")
- **objection**: blocking issues ("Benchmarks are missing") — blocks acceptance until resolved
- **evidence**: discussion about evidence ("See benchmark results in bench/")
- **summary**: the human-readable conclusion; required before RFC acceptance
- **action**: tasks to track ("Add div-by-zero guard") — blocks issue close until resolved
- **risk**: non-blocking concerns ("Floating-point precision may diverge")
- **review**: overall thread analysis, distinct from claim (single point) and summary (consensus)
- **alternative**: documents what was *not* chosen and why (especially in DEC threads)
- **assumption**: surfaces hidden dependencies that may invalidate the decision if they change

## Canonical form

```
git forum node add <THREAD> --type <TYPE> "body"
```

This works for all 10 node types. All accept a positional body argument, --body, --body-file,
--edit, --reply-to, and --as. Priority: positional > --body > --body-file.
Pass "-" as the positional body to read from stdin (e.g. `echo "body" | git forum claim ASK-0001 --body -`).
--edit opens $VISUAL / $EDITOR / vi for interactive composition (requires a TTY; conflicts with body args).
In scripts or agent workflows, use --body, --body-file, or --body - instead of --edit.

## Shorthand commands (2.0 canonical + deprecated aliases)

```
git forum comment <THREAD> "body"       # canonical 2.0 (node add --type comment)
git forum objection <THREAD> "body"     # node add --type objection
git forum action <THREAD> "body"        # node add --type action
```

The following shorthands are deprecated aliases for `comment` (ADR-006 / SPEC-2.0 §2.5);
they still work in 2.0 but emit a deprecation warning and will be removed in 3.0:

```
git forum claim, question, summary, risk, review  # deprecated → comment
```

Authors who relied on the rhetorical distinction express it in the body
(e.g. start with `Q:`, `Decision:`, `Risk:`).

alternative and assumption have no shorthand — use `node add --type <TYPE>` for these
(also deprecated to `comment` in 2.0 migration; see ADR-006).

## Reading a node's full body

`git forum node show <NODE_ID>` prints the full node body, type, actor, timestamp, and parent
thread context. Accepts full IDs or unique prefixes (8+ chars). Use this to read long review or
objection text that is truncated in the timeline.
"#
    .to_string()
}

/// State transition map for --help-llm.
pub fn state_transition_map() -> String {
    let mut out = String::new();
    out.push_str("# State Transition Map\n\n");

    for lifecycle in [Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record] {
        out.push_str(&format!("## {} transitions\n\n", lifecycle));
        out.push_str(&format!(
            "Initial state: `{}`\n\n",
            lifecycle.initial_state()
        ));
        out.push_str("| From | To |\n|------|----|\n");
        for (from, to) in state_machine::UNIFIED_TRANSITIONS {
            if lifecycle.allows_state(from) && lifecycle.allows_state(to) {
                out.push_str(&format!("| {from} | {to} |\n"));
            }
        }
        out.push('\n');
    }

    out.push_str("## Canonical form\n\n");
    out.push_str("```\n");
    out.push_str("git forum state <ID> <state>   # single grammar for all transitions\n");
    out.push_str("git forum state <ID> open       # thread reopen (done/rejected -> open)\n");
    out.push_str("```\n\n");
    out.push_str("State names are 2.0 canonical: `draft`, `open`, `working`, `review`, `done`,\n");
    out.push_str("`rejected`, `withdrawn`, `deprecated`. Reachability is keyed on the thread's\n");
    out.push_str("`lifecycle` facet (proposal/execution/record), not the legacy 1.x `kind`.\n\n");
    out.push_str("All accept --as, --comment, and --fast-track.\n");
    out.push_str(
        "`state` with done also accepts --approve, --link-to, --rel, --resolve-open-actions.\n",
    );
    out.push_str("--fast-track walks through intermediate states to reach the target, checking guards at each step.\n\n");
    out.push_str("## Shorthand commands (lifecycle-aware, SPEC-2.0 §9.3)\n\n");
    out.push_str("```\n");
    out.push_str(
        "git forum close <ID>       # execution/record: -> done; proposal: rejected (use accept)\n",
    );
    out.push_str(
        "git forum accept <ID>      # proposal/record: -> done; execution: rejected (use close)\n",
    );
    out.push_str(
        "git forum propose <ID>     # proposal: draft -> open; other lifecycles: rejected\n",
    );
    out.push_str(
        "git forum pend <ID>        # execution: -> working; other lifecycles: rejected\n",
    );
    out.push_str("git forum reject <ID>      # any lifecycle: -> rejected\n");
    out.push_str(
        "git forum withdraw <ID>    # proposal: -> withdrawn; other lifecycles: rejected\n",
    );
    out.push_str("git forum deprecate <ID>   # any lifecycle: -> deprecated\n");
    out.push_str("```\n\n");
    out.push_str("## Discoverability\n\n");
    out.push_str("`git forum show <ID>` includes a compact `next:` line and state diagram.\n");
    out.push_str(
        "`git forum show <ID> --what-next` shows guard checks and operation check rules.\n",
    );
    out.push_str("`git forum policy show` displays the full loaded policy.\n\n");
    out.push_str("## Lifecycle-scoped guard keys (2.0)\n\n");
    out.push_str("Guards support an optional `lifecycle:`/`tag:` prefix to scope a rule:\n");
    out.push_str("```toml\n");
    out.push_str("[[guards]]\n");
    out.push_str("on = \"lifecycle:record open->done\"  # only record-lifecycle threads\n");
    out.push_str("requires = [\"no_open_objections\"]\n\n");
    out.push_str("[[guards]]\n");
    out.push_str("on = \"open->done\"                    # all lifecycles (wildcard)\n");
    out.push_str("requires = [\"no_open_objections\"]\n");
    out.push_str("```\n");
    out.push_str("When both scoped and unscoped guards match, both apply (union).\n\n");
    out.push_str(
        "`git forum show <ID> --compact` truncates all sections to single-line previews.\n",
    );
    out.push_str("`git forum show <ID> --no-timeline` omits the timeline section.\n");
    out.push_str(
        "`git forum log <ID> --since <DATE>` shows only events after a date (ISO date, RFC 3339, or git revision).\n",
    );
    out.push_str("`git forum log <ID> -n <N>` limits output to the last N events.\n");
    out.push_str("`git forum log <ID> --type <TYPE>` filters by displayed event type (e.g. comment, state, action, revise-body).\n\n");
    out.push_str("## Hooks\n\n");
    out.push_str("`git forum hook install` installs commit-msg and post-checkout hooks.\n");
    out.push_str("`git forum hook fix-index` repairs missing blob references in the index.\n");
    out.push_str("`git forum hook worktree-init` auto-initializes git-forum in new worktrees.\n");

    out
}

/// Evidence kinds reference for --help-llm.
pub fn evidence_kinds_reference() -> String {
    r#"# Evidence Kinds

| Kind | Description | --ref value |
|------|-------------|-------------|
| commit | Git commit | SHA, branch, tag, HEAD, HEAD~1 |
| file | Source file | relative path |
| hunk | Specific code region | path with line range |
| test | Test file or suite | path to test file |
| benchmark | Performance data | path to benchmark output |
| doc | Documentation | path to doc file |
| thread | Another forum thread | thread ID |
| external | External URL or resource | URL or identifier |

## Usage

```
git forum evidence add <THREAD> --kind commit --ref HEAD
git forum evidence add <THREAD> --kind commit --ref abc123 def456
git forum evidence add <THREAD> --kind file --ref src/lib.rs
git forum evidence add <THREAD> --kind test --ref tests/backend_test.rs
git forum evidence add <THREAD> --kind benchmark --ref bench/result.csv
```

--ref accepts multiple values. Each creates its own evidence event.
For --kind commit, the revision is resolved to a canonical commit OID.
"#
    .to_string()
}
