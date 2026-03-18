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
| alternative | Propose a competing approach or design | no |
| evidence | Reference supporting data (distinct from evidence attachment) | no |
| summary | Consensus digest — what the thread concludes | no |
| action | A task to be completed (must be resolved before closing) | yes |
| risk | Flag a potential problem or uncertainty | no |
| assumption | State an assumption the proposal depends on | no |
| review | Holistic analysis of the entire thread | no |

## When to use each

- **claim**: single assertions ("We should use trait objects")
- **question**: requests for info ("What is the migration plan?")
- **objection**: blocking issues ("Benchmarks are missing") — blocks acceptance until resolved
- **alternative**: competing proposals ("Consider a stack-based approach instead")
- **evidence**: discussion about evidence ("See benchmark results in bench/")
- **summary**: the human-readable conclusion; required before RFC acceptance
- **action**: tasks to track ("Add div-by-zero guard") — blocks issue close until resolved
- **risk**: non-blocking concerns ("Floating-point precision may diverge")
- **assumption**: dependencies ("Assumes IEEE 754 doubles")
- **review**: overall thread analysis, distinct from claim (single point) and summary (consensus)

## Shorthand commands

```
git forum claim <THREAD> "body"
git forum question <THREAD> "body"
git forum objection <THREAD> "body"
git forum summary <THREAD> "body"
git forum action <THREAD> "body"
git forum risk <THREAD> "body"
git forum review <THREAD> "body"
```

All accept --body, --body-file, --body -, --reply-to, --as.
"#
    .to_string()
}

/// State transition map for --help-llm.
pub fn state_transition_map() -> String {
    let mut out = String::new();
    out.push_str("# State Transition Map\n\n");

    for kind in [ThreadKind::Issue, ThreadKind::Rfc] {
        out.push_str(&format!("## {} transitions\n\n", kind));
        out.push_str(&format!("Initial state: `{}`\n\n", kind.initial_status()));
        out.push_str("| From | To |\n|------|----|\n");
        for (from, to) in state_machine::valid_transitions(kind) {
            out.push_str(&format!("| {from} | {to} |\n"));
        }
        out.push('\n');
    }

    out.push_str("## Shorthand commands\n\n");
    out.push_str("```\n");
    out.push_str("git forum issue close <ID>       # open/pending -> closed\n");
    out.push_str("git forum issue pend <ID>        # open -> pending\n");
    out.push_str("git forum issue reopen <ID>      # closed/rejected -> open\n");
    out.push_str("git forum issue reject <ID>      # open -> rejected\n");
    out.push_str("git forum rfc propose <ID>       # draft -> proposed\n");
    out.push_str("git forum rfc accept <ID>        # under-review -> accepted\n");
    out.push_str("git forum rfc deprecate <ID>     # accepted/rejected -> deprecated\n");
    out.push_str("```\n\n");
    out.push_str("All accept --sign, --comment, --link-to, --rel, --resolve-open-actions.\n");
    out.push_str(
        "Use `git forum show <ID> --what-next` to see valid transitions and guard status.\n",
    );

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
