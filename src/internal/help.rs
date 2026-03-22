use super::event::ThreadKind;
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
Pass "-" as the positional body to read from stdin (e.g. `echo "body" | git forum claim ISSUE-0001 --body -`).
--edit opens $VISUAL / $EDITOR / vi for interactive composition (requires a TTY; conflicts with body args).
In scripts or agent workflows, use --body, --body-file, or --body - instead of --edit.

## Shorthand commands (convenience aliases)

```
git forum claim <THREAD> "body"         # node add --type claim
git forum question <THREAD> "body"      # node add --type question
git forum objection <THREAD> "body"     # node add --type objection
git forum summary <THREAD> "body"       # node add --type summary
git forum action <THREAD> "body"        # node add --type action
git forum risk <THREAD> "body"          # node add --type risk
git forum review <THREAD> "body"        # node add --type review
```

alternative and assumption have no shorthand — use `node add --type <TYPE>` for these.

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

    for kind in [
        ThreadKind::Issue,
        ThreadKind::Rfc,
        ThreadKind::Dec,
        ThreadKind::Task,
    ] {
        out.push_str(&format!("## {} transitions\n\n", kind));
        out.push_str(&format!("Initial state: `{}`\n\n", kind.initial_status()));
        out.push_str("| From | To |\n|------|----|\n");
        for (from, to) in state_machine::valid_transitions(kind) {
            out.push_str(&format!("| {from} | {to} |\n"));
        }
        out.push('\n');
    }

    out.push_str("## Canonical form\n\n");
    out.push_str("```\n");
    out.push_str("git forum state <ID> <state>   # single grammar for all transitions\n");
    out.push_str("git forum state <ID> open       # thread reopen (closed/rejected -> open)\n");
    out.push_str("```\n\n");
    out.push_str("For TASK phase transitions: `git forum state <ID> <state>` with:\n");
    out.push_str("designing, implementing, reviewing.\n\n");
    out.push_str("All accept --as, --comment, and --fast-track.\n");
    out.push_str("`state` with closed/accepted also accepts --approve, --link-to, --rel.\n");
    out.push_str("`state` with closed also accepts --resolve-open-actions.\n");
    out.push_str("--fast-track walks through intermediate states to reach the target, checking guards at each step.\n\n");
    out.push_str("## Shorthand commands (convenience aliases)\n\n");
    out.push_str("```\n");
    out.push_str("git forum close <ID>       # state <ID> closed\n");
    out.push_str("git forum pend <ID>        # state <ID> pending\n");
    out.push_str("git forum reject <ID>      # state <ID> rejected\n");
    out.push_str("git forum propose <ID>     # state <ID> proposed\n");
    out.push_str("git forum accept <ID>      # state <ID> accepted\n");
    out.push_str("git forum deprecate <ID>   # state <ID> deprecated\n");
    out.push_str("```\n\n");
    out.push_str("## Discoverability\n\n");
    out.push_str("`git forum show <ID>` includes a compact `next:` line and state diagram.\n");
    out.push_str(
        "`git forum show <ID> --what-next` shows guard checks and operation check rules.\n",
    );
    out.push_str("`git forum policy show` displays the full loaded policy.\n\n");
    out.push_str("## Kind-scoped guard keys\n\n");
    out.push_str("Guards support an optional kind prefix to restrict to a specific thread kind:\n");
    out.push_str("```toml\n");
    out.push_str("[[guards]]\n");
    out.push_str("on = \"dec:proposed->accepted\"  # only DEC threads\n");
    out.push_str("requires = [\"no_open_objections\"]\n\n");
    out.push_str("[[guards]]\n");
    out.push_str("on = \"proposed->accepted\"      # all kinds (wildcard)\n");
    out.push_str("requires = [\"no_open_objections\"]\n");
    out.push_str("```\n");
    out.push_str("When both scoped and unscoped guards match, both apply (union).\n\n");
    out.push_str(
        "`git forum show <ID> --compact` truncates all sections to single-line previews.\n",
    );
    out.push_str("`git forum show <ID> --no-timeline` omits the timeline section.\n");

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
