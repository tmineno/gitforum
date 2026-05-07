use super::policy::CategoryRegistry;

/// Node type taxonomy for `--help-llm` (SPEC-3.0 §2.2).
pub fn node_type_taxonomy() -> String {
    r#"# Node Type Taxonomy

SPEC-3.0 §2.2: discussion uses four canonical node kinds,
chosen by protocol effect.

| Kind | Purpose | Resolvable |
|------|---------|------------|
| comment | Free-form prose contribution | no |
| objection | Raise a blocking concern (must be resolved before forward transitions) | yes |
| approval | Record approval, typically via `--approve <actor>` on a state transition | no |
| action | A task to be completed (must be resolved before terminal states) | yes |

## When to use each

- **comment**: any prose contribution. Authors who want a rhetorical
  prefix (claim, question, summary, risk, alternative, assumption)
  express it in the body (e.g. start with `Q:`, `Decision:`, `Risk:`).
- **objection**: blocking issues ("Benchmarks are missing") — gates
  the `no_open_objections` policy guard until resolved or retracted.
- **approval**: surfaces a `--approve <actor>` decision; counted by
  the `one_approval` guard (any actor type per SPEC-3.0 §3.2).
- **action**: tasks to track ("Add div-by-zero guard") — must be
  resolved before the thread can move to a terminal state.

## Canonical form

```
git forum node add <THREAD> --type <TYPE> "body"
```

Accepts positional body, `--body`, `--body-file`, `--edit`,
`--reply-to`, and `--as`. Priority: positional > `--body` >
`--body-file`. Both `--body -` and `--body-file -` read from
stdin (ticket `h36at1ti`). `--edit` opens `$VISUAL` / `$EDITOR`
/ `vi` (requires a TTY; conflicts with body args, including the
stdin `-` form). In scripts or agent workflows, prefer `--body`,
`--body-file`, or pipe content via `--body -`.

## Shorthand commands

```
git forum comment   <THREAD> "body"     # node add --type comment
git forum objection <THREAD> "body"     # node add --type objection
git forum action    <THREAD> "body"     # node add --type action
```

There is no `approval` shorthand. Approvals are recorded by passing
`--approve <actor>` on a state-transition command (e.g.
`git forum accept <ID> --approve human/alice`).

The 1.x rhetorical aliases (`claim`, `question`, `summary`, `risk`,
`review`, `alternative`, `assumption`) are not 3.0 subcommands.
Threads migrated from 1.x/2.x preserve them as a `legacy_label`
field on the resulting `comment` nodes (SPEC-3.0 §8.1).

## Reading a node's full body

`git forum node show <NODE_ID>` prints the full node body, kind,
actor, timestamp, and parent thread context. Accepts full IDs or
unique prefixes (8+ chars). Use this to read long objection or
comment text truncated in `git forum show`.

## Storage shape (SPEC-3.0 §4.2)

Each node writes `nodes/<node_id>.toml` (kind, status, created_*,
updated_*, reply_to, optional legacy_label) and `nodes/<node_id>.md`
(body) on the thread ref `refs/forum/threads/<id>`. Mutating a node
status (`resolve`, `retract`, `reopen`) rewrites only the `.toml`;
revising body rewrites the `.md`. Revision history is `git log`
over the ref.
"#
    .to_string()
}

/// State transition map for `--help-llm` (SPEC-3.0 §3.1).
pub fn state_transition_map() -> String {
    let mut out = String::new();
    out.push_str("# State Transition Map\n\n");
    out.push_str(
        "SPEC-3.0 §3.1: each thread carries a `category` (`rfc` or `task`)\n\
         that defines the legal status set and the legal transitions. The\n\
         status names are unified across categories; per-category\n\
         restrictions appear in the tables below.\n\n",
    );

    let registry = CategoryRegistry::built_in();
    let mut category_names: Vec<&String> = registry.categories.keys().collect();
    category_names.sort();
    for name in category_names {
        let def = registry.get(name).expect("name from keys()");
        out.push_str(&format!("## `category = \"{name}\"`\n\n"));
        out.push_str(&format!("Initial status: `{}`\n\n", def.initial_status));
        out.push_str("| From | To |\n|------|----|\n");
        let mut transitions: Vec<&String> = def.transitions.iter().collect();
        transitions.sort();
        for t in transitions {
            if let Some((from, to)) = t.split_once("->") {
                out.push_str(&format!("| {from} | {to} |\n"));
            }
        }
        out.push('\n');
    }

    out.push_str("## Canonical form\n\n");
    out.push_str("```\n");
    out.push_str("git forum state <ID> <STATUS>     # single grammar for all transitions\n");
    out.push_str("git forum state bulk --to <STATUS> <ID>...\n");
    out.push_str("git forum reopen <THREAD_ID>      # closed → open (single thread id)\n");
    out.push_str("```\n\n");
    out.push_str(
        "Status names are SPEC-3.0 canonical: `draft`, `open`, `working`,\n\
         `review`, `done`, `rejected`, `withdrawn`, `deprecated`. Reachability\n\
         is keyed on the thread's `category`, not the legacy 1.x `kind`.\n\n",
    );
    out.push_str(
        "All transitions accept `--as`, `--comment`, and `--fast-track`.\n\
         `state` (and shorthands like `accept` / `close`) also accept\n\
         `--approve`, `--link-to`, `--rel`, and `--resolve-open-actions`.\n\
         `--fast-track` walks intermediate states, checking guards at each step.\n\n",
    );

    out.push_str("## Shorthand commands (category-aware)\n\n");
    out.push_str("```\n");
    out.push_str("git forum close    <ID>   # task: -> done; rfc: rejected (use accept)\n");
    out.push_str("git forum accept   <ID>   # rfc: -> done; task: rejected (use close)\n");
    out.push_str("git forum propose  <ID>   # rfc: draft -> open; task: rejected\n");
    out.push_str("git forum pend     <ID>   # task: -> working; rfc: rejected\n");
    out.push_str("git forum reject   <ID>   # any category: -> rejected\n");
    out.push_str("git forum withdraw <ID>   # rfc: -> withdrawn; task: rejected\n");
    out.push_str("git forum deprecate <ID>  # any category: -> deprecated\n");
    out.push_str(
        "git forum supersede <OLD> --by <NEW>  # link superseded-by + comment + -> deprecated\n",
    );
    out.push_str("```\n\n");
    out.push_str(
        "`supersede` collapses the three-step recipe (link `<old>`\n\
         `superseded-by` `<new>`, attach a comment on `<old>`, transition\n\
         `<old>` to `deprecated`) into one verb. Default comment body is\n\
         `Superseded by @<new>` if `--body` is omitted. Lands `<old>` in\n\
         `deprecated` (not `rejected`), so superseded threads do not\n\
         appear in `git forum ls --status rejected`. Also writes the\n\
         symmetric `supersedes` link on `<new>` so `git forum show <NEW>`\n\
         surfaces the relationship.\n\n",
    );

    out.push_str("## Discoverability\n\n");
    out.push_str("`git forum show <ID>` prints a compact `next:` line and state diagram.\n");
    out.push_str(
        "`git forum show <ID> --what-next` shows guard checks and operation-check rules.\n",
    );
    out.push_str("`git forum verify <ID>` reports which guards block the next forward target.\n");
    out.push_str("`git forum policy show` displays the loaded policy.\n");
    out.push_str("`git forum status <ID>` reports unresolved items (objections, actions, evidence gaps).\n\n");

    out.push_str("## Guards (SPEC-3.0 §3.2)\n\n");
    out.push_str(
        "Guards are attached directly to a category transition under\n\
         `[categories.<NAME>.guards]`, keyed by the `\"FROM->TO\"`\n\
         transition string. The value is the list of guard rules:\n\n",
    );
    out.push_str("```toml\n");
    out.push_str("[categories.rfc.guards]\n");
    out.push_str("\"review->done\" = [\"one_approval\", \"no_open_objections\"]\n\n");
    out.push_str("[categories.task.guards]\n");
    out.push_str("\"working->done\" = [\"no_open_actions\", \"no_open_objections\"]\n");
    out.push_str("```\n\n");
    out.push_str(
        "Rule names: `no_open_objections`, `no_open_actions`, `one_approval`,\n\
         `has_commit_evidence`. 3.0 has no tag/lifecycle selector language\n\
         (§3.1: \"3.0 does not define a selector language over tags or other\n\
         facets\").\n\n",
    );
    out.push_str(
        "Per-category operation checks live under the same\n\
         `[categories.<NAME>.*]` umbrella: `creation`, `allowed_node_types`,\n\
         `revise`, `evidence` (SPEC-3.0 §3.3). Loading a v2-form\n\
         `policy.toml` (`[[guards]]`, `requires =`, kind/lifecycle/facet-\n\
         scoped `creation_rules.*`, `node_rules`, `revise_rules`,\n\
         `evidence_rules`, `one_human_approval`, `at_least_one_summary`)\n\
         is rejected with a hint pointing at `git forum migrate --to 3.0`.\n\n",
    );
    out.push_str("## Storage shape (SPEC-3.0 §4.2)\n\n");
    out.push_str(
        "A state transition rewrites `thread.toml`'s `status`, `updated_at`,\n\
         and `updated_by` fields and creates one new commit on\n\
         `refs/forum/threads/<id>`. There is no separate event log;\n\
         `git log` over the ref is the audit trail.\n",
    );

    out
}

/// Evidence kinds reference for `--help-llm`.
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
git forum evidence add <THREAD> --kind commit --ref abc123 --ref def456
git forum evidence add <THREAD> --kind file --ref src/lib.rs
git forum evidence add <THREAD> --kind test --ref tests/backend_test.rs
git forum evidence add <THREAD> --kind benchmark --ref bench/result.csv
```

`--ref` accepts multiple values; each writes its own row to
`evidence.toml` on the thread ref. For `--kind commit`, the
revision is resolved through `git rev-parse` to a canonical 40-char
OID before storing — `--ref HEAD` becomes the resolved SHA.

## Storage shape (SPEC-3.0 §4.2)

Each `evidence add` rewrites `evidence.toml` (one row per `--ref`)
and creates one commit on `refs/forum/threads/<id>`. Policy guards
that check evidence (e.g. `has_commit_evidence`) read the rows
directly from `evidence.toml`; there is no separate event log.
"#
    .to_string()
}
