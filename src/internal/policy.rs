use std::collections::HashMap;

use serde::Deserialize;

use super::approval::Approval;
use super::error::{ForumError, ForumResult};
use super::event::{NodeType, ThreadKind};
use super::evidence::EvidenceKind;
use super::state_machine;
use super::thread::ThreadState;

/// A named guard rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardRule {
    NoOpenObjections,
    NoOpenActions,
    AtLeastOneSummary,
    OneHumanApproval,
    HasCommitEvidence,
}

impl std::fmt::Display for GuardRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOpenObjections => write!(f, "no_open_objections"),
            Self::NoOpenActions => write!(f, "no_open_actions"),
            Self::AtLeastOneSummary => write!(f, "at_least_one_summary"),
            Self::OneHumanApproval => write!(f, "one_human_approval"),
            Self::HasCommitEvidence => write!(f, "has_commit_evidence"),
        }
    }
}

/// A guard entry: a set of rules that must pass for a given transition.
#[derive(Debug, Clone, Deserialize)]
pub struct GuardEntry {
    pub on: String,
    pub requires: Vec<GuardRule>,
}

/// Creation rules for a specific thread kind (e.g. rfc, issue).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreationRules {
    #[serde(default)]
    pub required_body: bool,
    #[serde(default)]
    pub body_sections: Vec<String>,
}

/// Rules controlling which states allow body/node revisions.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReviseRules {
    #[serde(default)]
    pub allow_body_revise: Vec<String>,
    #[serde(default)]
    pub allow_node_revise: Vec<String>,
}

/// Rules controlling which states allow evidence addition.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvidenceRules {
    #[serde(default)]
    pub allow_evidence: Vec<String>,
}

/// Global check settings.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChecksConfig {
    #[serde(default)]
    pub strict: bool,
}

/// Parsed policy loaded from `.forum/policy.toml`.
///
/// Preconditions: none (loaded from file).
/// Postconditions: guards are populated from TOML.
/// Failure modes: ForumError::Config on parse failure.
/// Side effects: none (read-only after load).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Policy {
    #[serde(default, rename = "guards")]
    pub guards: Vec<GuardEntry>,
    #[serde(default)]
    pub creation_rules: HashMap<String, CreationRules>,
    #[serde(default)]
    pub node_rules: HashMap<String, Vec<NodeType>>,
    #[serde(default)]
    pub revise_rules: Option<ReviseRules>,
    #[serde(default)]
    pub evidence_rules: Option<EvidenceRules>,
    #[serde(default)]
    pub checks: ChecksConfig,
}

impl Policy {
    /// Load and parse policy from the given path.
    pub fn load(path: &std::path::Path) -> ForumResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ForumError::Config(format!("cannot read policy.toml: {e}")))?;
        toml::from_str(&text).map_err(|e| ForumError::Config(format!("invalid policy.toml: {e}")))
    }

    /// Return guards that apply to the given transition string (e.g. `"under-review->accepted"`).
    pub fn guards_for(&self, transition: &str) -> Vec<&GuardEntry> {
        self.guards.iter().filter(|g| g.on == transition).collect()
    }
}

/// A guard violation: a rule that was not satisfied.
#[derive(Debug, Clone)]
pub struct GuardViolation {
    pub rule: String,
    pub reason: String,
}

/// Evaluate all guards for a transition and return any violations.
///
/// Preconditions: state is fully replayed; approvals from the proposed event.
/// Postconditions: empty vec means all guards pass.
/// Failure modes: none (returns violations, not errors).
/// Side effects: none.
pub fn check_guards(
    policy: &Policy,
    state: &ThreadState,
    from: &str,
    to: &str,
    approvals: &[Approval],
) -> Vec<GuardViolation> {
    let transition = format!("{from}->{to}");
    let mut violations = Vec::new();
    for guard in policy.guards_for(&transition) {
        for rule in &guard.requires {
            if let Some(v) = evaluate_rule(rule, state, approvals) {
                violations.push(v);
            }
        }
    }
    violations
}

/// Evaluate a single guard rule. Returns `Some(violation)` if the rule is not satisfied.
pub fn evaluate_rule(
    rule: &GuardRule,
    state: &ThreadState,
    approvals: &[Approval],
) -> Option<GuardViolation> {
    match rule {
        GuardRule::NoOpenObjections => {
            let open = state.open_objections();
            if !open.is_empty() {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: format!("{} open objection(s)", open.len()),
                })
            } else {
                None
            }
        }
        GuardRule::NoOpenActions => {
            let open = state.open_actions();
            if !open.is_empty() {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: format!("{} open action(s)", open.len()),
                })
            } else {
                None
            }
        }
        GuardRule::AtLeastOneSummary => {
            if state.latest_summary().is_none() {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: "no summary node found".into(),
                })
            } else {
                None
            }
        }
        GuardRule::OneHumanApproval => {
            let has_human = approvals.iter().any(|a| a.actor_id.starts_with("human/"));
            if !has_human {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: "no human approval recorded".into(),
                })
            } else {
                None
            }
        }
        GuardRule::HasCommitEvidence => {
            let has_commit = state
                .evidence_items
                .iter()
                .any(|e| e.kind == EvidenceKind::Commit);
            if !has_commit {
                Some(GuardViolation {
                    rule: rule.to_string(),
                    reason: "no commit evidence attached".into(),
                })
            } else {
                None
            }
        }
    }
}

/// Severity level for a lint diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// Informational note (e.g. multi-kind transition — intentional but worth knowing).
    Note,
    /// Warning about a likely mistake (e.g. unknown state name).
    Warn,
}

/// A single lint diagnostic with a severity level.
#[derive(Debug, Clone)]
pub struct LintDiag {
    pub level: LintLevel,
    pub message: String,
}

impl std::fmt::Display for LintDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self.level {
            LintLevel::Note => "NOTE",
            LintLevel::Warn => "WARN",
        };
        write!(f, "{prefix} {}", self.message)
    }
}

/// Lint a policy for structural problems.
///
/// Preconditions: policy is loaded.
/// Postconditions: returns a list of diagnostics (empty = OK).
/// Failure modes: none.
/// Side effects: none.
pub fn lint_policy(policy: &Policy) -> Vec<LintDiag> {
    let mut diags = Vec::new();
    let all_kinds = [
        ThreadKind::Issue,
        ThreadKind::Rfc,
        ThreadKind::Dec,
        ThreadKind::Task,
    ];

    // Collect all valid states across all kinds for global validation.
    let all_states: std::collections::HashSet<&str> = all_kinds
        .iter()
        .flat_map(|k| {
            state_machine::valid_transitions(*k)
                .iter()
                .flat_map(|(from, to)| [*from, *to])
        })
        .collect();

    for guard in &policy.guards {
        if !guard.on.contains("->") {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard 'on' field {:?} is not a valid transition (expected 'from->to')",
                    guard.on
                ),
            });
            continue;
        }

        let parts: Vec<&str> = guard.on.splitn(2, "->").collect();
        let (from, to) = (parts[0], parts[1]);

        // Check for undefined states.
        if !all_states.contains(from) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: unknown state {:?}; valid states are: {}",
                    guard.on,
                    from,
                    sorted_states(&all_states),
                ),
            });
        }
        if !all_states.contains(to) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: unknown state {:?}; valid states are: {}",
                    guard.on,
                    to,
                    sorted_states(&all_states),
                ),
            });
        }

        // Check which kinds this transition applies to and note if multiple.
        let matching_kinds: Vec<&str> = all_kinds
            .iter()
            .filter(|k| state_machine::is_valid_transition(**k, from, to))
            .map(|k| kind_name(k))
            .collect();
        if matching_kinds.len() > 1 {
            diags.push(LintDiag {
                level: LintLevel::Note,
                message: format!(
                    "guard {:?}: transition applies to multiple thread kinds ({}); \
                     use kind-unique states if you need different rules per kind",
                    guard.on,
                    matching_kinds.join(", "),
                ),
            });
        } else if matching_kinds.is_empty() && all_states.contains(from) && all_states.contains(to)
        {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: not a valid transition for any thread kind",
                    guard.on,
                ),
            });
        }
    }

    // Validate state names in operation checks.
    lint_state_list(
        &mut diags,
        &all_states,
        "revise_rules.allow_body_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_body_revise.as_slice())
            .unwrap_or(&[]),
    );
    lint_state_list(
        &mut diags,
        &all_states,
        "revise_rules.allow_node_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_node_revise.as_slice())
            .unwrap_or(&[]),
    );
    lint_state_list(
        &mut diags,
        &all_states,
        "evidence_rules.allow_evidence",
        policy
            .evidence_rules
            .as_ref()
            .map(|r| r.allow_evidence.as_slice())
            .unwrap_or(&[]),
    );

    // Validate state names in node_rules keys.
    for state_name in policy.node_rules.keys() {
        if !all_states.contains(state_name.as_str()) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "node_rules: unknown state {:?}; valid states are: {}",
                    state_name,
                    sorted_states(&all_states),
                ),
            });
        }
    }

    diags
}

fn lint_state_list(
    diags: &mut Vec<LintDiag>,
    all_states: &std::collections::HashSet<&str>,
    field: &str,
    states: &[String],
) {
    for s in states {
        if !all_states.contains(s.as_str()) {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "{field}: unknown state {:?}; valid states are: {}",
                    s,
                    sorted_states(all_states),
                ),
            });
        }
    }
}

fn sorted_states(states: &std::collections::HashSet<&str>) -> String {
    let mut v: Vec<&str> = states.iter().copied().collect();
    v.sort_unstable();
    v.join(", ")
}

fn kind_name(kind: &ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Issue => "issue",
        ThreadKind::Rfc => "rfc",
        ThreadKind::Dec => "dec",
        ThreadKind::Task => "task",
    }
}

/// Render the policy in human-readable format for `policy show`.
///
/// Only shows sections that are actually configured — no synthesized defaults.
pub fn render_policy_show(policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Guards
    if !policy.guards.is_empty() {
        lines.push("guards:".into());
        for guard in &policy.guards {
            let rules: Vec<String> = guard.requires.iter().map(|r| r.to_string()).collect();
            lines.push(format!("  {}: {}", guard.on, rules.join(", ")));
        }
        lines.push(String::new());
    }

    // Checks
    lines.push("checks:".into());
    lines.push(format!("  strict = {}", policy.checks.strict));
    lines.push(String::new());

    // Creation rules
    if !policy.creation_rules.is_empty() {
        lines.push("creation_rules:".into());
        let mut keys: Vec<&String> = policy.creation_rules.keys().collect();
        keys.sort();
        for key in keys {
            let rules = &policy.creation_rules[key];
            let mut parts: Vec<String> = Vec::new();
            if rules.required_body {
                parts.push("required_body=true".into());
            }
            if !rules.body_sections.is_empty() {
                parts.push(format!("sections=[{}]", rules.body_sections.join(", ")));
            }
            if parts.is_empty() {
                lines.push(format!("  {key}: (no restrictions)"));
            } else {
                lines.push(format!("  {key}: {}", parts.join(", ")));
            }
        }
        lines.push(String::new());
    }

    // Node rules
    if !policy.node_rules.is_empty() {
        lines.push("node_rules:".into());
        let mut keys: Vec<&String> = policy.node_rules.keys().collect();
        keys.sort();
        for key in keys {
            let types: Vec<String> = policy.node_rules[key]
                .iter()
                .map(|n| n.to_string())
                .collect();
            if types.is_empty() {
                lines.push(format!("  {key}: (none allowed)"));
            } else {
                lines.push(format!("  {key}: {}", types.join(", ")));
            }
        }
        lines.push(String::new());
    } else {
        lines.push("node_rules: (not configured)".into());
        lines.push(String::new());
    }

    // Revise rules
    if let Some(revise) = &policy.revise_rules {
        lines.push("revise_rules:".into());
        if !revise.allow_body_revise.is_empty() {
            lines.push(format!("  body: [{}]", revise.allow_body_revise.join(", ")));
        }
        if !revise.allow_node_revise.is_empty() {
            lines.push(format!("  node: [{}]", revise.allow_node_revise.join(", ")));
        }
        lines.push(String::new());
    } else {
        lines.push("revise_rules: (not configured)".into());
        lines.push(String::new());
    }

    // Evidence rules
    if let Some(evidence) = &policy.evidence_rules {
        lines.push("evidence_rules:".into());
        lines.push(format!("  allow: [{}]", evidence.allow_evidence.join(", ")));
        lines.push(String::new());
    } else {
        lines.push("evidence_rules: (not configured)".into());
        lines.push(String::new());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_policy() -> Policy {
        Policy {
            guards: vec![GuardEntry {
                on: "under-review->accepted".into(),
                requires: vec![
                    GuardRule::NoOpenObjections,
                    GuardRule::NoOpenActions,
                    GuardRule::AtLeastOneSummary,
                    GuardRule::OneHumanApproval,
                ],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn guards_for_matches_transition() {
        let policy = minimal_policy();
        assert_eq!(policy.guards_for("under-review->accepted").len(), 1);
        assert!(policy.guards_for("draft->under-review").is_empty());
    }

    #[test]
    fn lint_valid_policy_returns_empty() {
        let policy = minimal_policy();
        assert!(lint_policy(&policy).is_empty());
    }

    #[test]
    fn lint_invalid_transition_reports_diag() {
        let policy = Policy {
            guards: vec![GuardEntry {
                on: "badvalue".into(),
                requires: vec![],
            }],
            ..Default::default()
        };
        assert!(!lint_policy(&policy).is_empty());
    }

    #[test]
    fn guard_rule_display() {
        assert_eq!(
            GuardRule::NoOpenObjections.to_string(),
            "no_open_objections"
        );
        assert_eq!(
            GuardRule::AtLeastOneSummary.to_string(),
            "at_least_one_summary"
        );
        assert_eq!(GuardRule::NoOpenActions.to_string(), "no_open_actions");
        assert_eq!(
            GuardRule::OneHumanApproval.to_string(),
            "one_human_approval"
        );
        assert_eq!(
            GuardRule::HasCommitEvidence.to_string(),
            "has_commit_evidence"
        );
    }

    #[test]
    fn lint_unknown_guard_from_state() {
        let policy = Policy {
            guards: vec![GuardEntry {
                on: "bogus->closed".into(),
                requires: vec![],
            }],
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.level == LintLevel::Warn && d.message.contains("unknown state \"bogus\"")));
    }

    #[test]
    fn lint_unknown_guard_to_state() {
        let policy = Policy {
            guards: vec![GuardEntry {
                on: "open->fantasy".into(),
                requires: vec![],
            }],
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unknown state \"fantasy\"")));
    }

    #[test]
    fn lint_notes_multi_kind_transition() {
        // "open->closed" applies to both issue and task
        let policy = Policy {
            guards: vec![GuardEntry {
                on: "open->closed".into(),
                requires: vec![GuardRule::NoOpenActions],
            }],
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let multi = diags
            .iter()
            .find(|d| d.message.contains("multiple thread kinds"));
        assert!(multi.is_some());
        assert_eq!(multi.unwrap().level, LintLevel::Note);
        assert!(multi.unwrap().message.contains("issue"));
    }

    #[test]
    fn lint_no_note_for_kind_unique_transition() {
        // "under-review->accepted" is RFC-only
        let policy = minimal_policy();
        let diags = lint_policy(&policy);
        assert!(!diags
            .iter()
            .any(|d| d.message.contains("multiple thread kinds")));
    }

    #[test]
    fn lint_invalid_transition_for_any_kind() {
        // "draft->closed" is valid states but not a valid transition
        let policy = Policy {
            guards: vec![GuardEntry {
                on: "draft->closed".into(),
                requires: vec![],
            }],
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message
                .contains("not a valid transition for any thread kind")));
    }

    #[test]
    fn lint_unknown_state_in_revise_rules() {
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec!["nonexistent".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("revise_rules.allow_body_revise")
                && d.message.contains("unknown state \"nonexistent\"")));
    }

    #[test]
    fn lint_unknown_state_in_node_rules_key() {
        let mut node_rules = HashMap::new();
        node_rules.insert("imaginary".to_string(), vec![]);
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.message.contains("node_rules")
            && d.message.contains("unknown state \"imaginary\"")));
    }

    #[test]
    fn lint_unknown_state_in_evidence_rules() {
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec!["nope".into()],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.message.contains("evidence_rules")
            && d.message.contains("unknown state \"nope\"")));
    }

    #[test]
    fn lint_default_policy_has_no_warnings() {
        let policy: Policy =
            toml::from_str(include_str!("../../tests/fixtures/policy_default.toml")).unwrap();
        let diags = lint_policy(&policy);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn)
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn render_policy_show_empty_policy() {
        let policy = Policy::default();
        let out = render_policy_show(&policy);
        assert!(out.contains("checks:"));
        assert!(out.contains("strict = false"));
        assert!(out.contains("node_rules: (not configured)"));
        assert!(out.contains("revise_rules: (not configured)"));
        assert!(out.contains("evidence_rules: (not configured)"));
    }

    #[test]
    fn render_policy_show_full_policy() {
        let policy: Policy =
            toml::from_str(include_str!("../../tests/fixtures/policy_default.toml")).unwrap();
        let out = render_policy_show(&policy);
        assert!(out.contains("guards:"));
        assert!(out.contains("under-review->accepted:"));
        assert!(out.contains("creation_rules:"));
        assert!(out.contains("checks:"));
    }
}
