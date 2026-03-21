# Feature: Operation Checks

## Goal

Validate all write operations against policy rules before committing events,
catching violations at the CLI boundary rather than only at state transitions.

## Non-goals

- New thread kinds or node types
- AI-specific policy profiles
- Changing existing state machines

## Inputs/Outputs

### Policy configuration (`.forum/policy.toml`)

The default policy shipped by `git forum init`:

```toml
[checks]
strict = false

[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Context", "Proposal"]

[creation_rules.issue]
required_body = false
body_sections = []

[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending"]

[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending", "closed", "accepted", "deprecated"]
```

Example of a stricter per-project variant (more sections, node restrictions, no terminal-state evidence):

```toml
[checks]
strict = false

[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals", "Design", "Failure modes", "Acceptance tests"]

[creation_rules.issue]
required_body = false
body_sections = []

[node_rules]
"draft" = ["claim", "question", "objection", "evidence", "summary", "action", "risk", "review"]
"accepted" = []

[revise_rules]
allow_body_revise = ["draft", "proposed", "open", "pending"]
allow_node_revise = ["draft", "proposed", "under-review", "open", "pending"]

[evidence_rules]
allow_evidence = ["draft", "proposed", "under-review", "open", "pending"]
```

### Check functions

| Function | Covers | What it checks |
|----------|--------|----------------|
| `check_create(policy, kind, title, body)` | `new` | Required body, body sections |
| `check_say(policy, status, node_type)` | node commands | Node type allowed in state |
| `check_revise(policy, status, is_body)` | `revise` | Revision allowed in state |
| `check_evidence(policy, status)` | `evidence add` | Evidence allowed in state |

Each returns `Vec<OperationViolation>`.

### OperationViolation

```rust
pub struct OperationViolation {
    pub severity: Severity,       // Error or Warning
    pub rule: String,             // machine-readable rule name
    pub reason: String,           // human-readable explanation
    pub hint: Option<String>,     // suggested fix text
    pub fix_command: Option<String>,
}
```

### Severity rules

- `Error`: always blocks the operation; `--force` does NOT bypass
- `Warning`: printed to stderr, operation proceeds
  - With `strict = true`: warnings become errors unless `--force`
  - With `--force` + `strict = true`: warnings downgrade back to warnings

### Specific severity assignments

- Missing body when `required_body = true`: **Error**
- Missing required section: **Warning**
- Empty required section: **Warning**
- Node type not allowed in state: **Error**
- Revision not allowed in state: **Error**
- Evidence not allowed in state: **Error**

## Failure modes

- Missing policy file: `Policy::default()` — all checks pass, zero violations
- Missing policy sections: `#[serde(default)]` — no restrictions for that check
- Over-restrictive rules: customize or remove; without `strict`, violations are warnings only
- `--force` misuse: never silently skips validation — violations always printed

## Performance/Safety

- Check functions are pure: policy + state in, violations out
- No additional thread replay — reuses state already replayed for the operation
- Backward compatible: all new Policy fields use `#[serde(default)]`

## Acceptance tests

- `new rfc "Test"` with no body + `required_body = true` → Error, blocked
- `new rfc "Test" --body "## Goal\nfoo"` → Warning for missing sections
- `claim RFC-XXXX "text"` on accepted RFC + `node_rules.accepted = []` → blocked
- `revise body RFC-XXXX` on accepted RFC + `revise_rules` excludes accepted → blocked
- `evidence add` on deprecated RFC + `evidence_rules` excludes deprecated → blocked
- `--force` does NOT bypass Error violations
- `strict = true` + warning → becomes error; `--force` downgrades back
- No policy file → all operations allowed (current behavior preserved)
- All existing tests pass unchanged
