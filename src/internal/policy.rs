use std::collections::HashMap;

use serde::Deserialize;

use super::approval::Approval;
use super::error::{ForumError, ForumResult};
use super::evidence::EvidenceKind;
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

/// Role definition from `policy.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyRole {
    #[serde(default)]
    pub can_say: Vec<String>,
    #[serde(default)]
    pub can_transition: Vec<String>,
}

/// Parsed policy loaded from `.forum/policy.toml`.
///
/// Preconditions: none (loaded from file).
/// Postconditions: guards and roles are populated from TOML.
/// Failure modes: ForumError::Config on parse failure.
/// Side effects: none (read-only after load).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub roles: HashMap<String, PolicyRole>,
    #[serde(default, rename = "guards")]
    pub guards: Vec<GuardEntry>,
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

/// Lint a policy for structural problems.
///
/// Preconditions: policy is loaded.
/// Postconditions: returns a list of diagnostic strings (empty = OK).
/// Failure modes: none.
/// Side effects: none.
pub fn lint_policy(policy: &Policy) -> Vec<String> {
    let mut diags = Vec::new();
    for guard in &policy.guards {
        if !guard.on.contains("->") {
            diags.push(format!(
                "guard 'on' field {:?} is not a valid transition (expected 'from->to')",
                guard.on
            ));
        }
    }
    diags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_policy() -> Policy {
        Policy {
            roles: HashMap::new(),
            guards: vec![GuardEntry {
                on: "under-review->accepted".into(),
                requires: vec![
                    GuardRule::NoOpenObjections,
                    GuardRule::NoOpenActions,
                    GuardRule::AtLeastOneSummary,
                    GuardRule::OneHumanApproval,
                ],
            }],
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
            roles: HashMap::new(),
            guards: vec![GuardEntry {
                on: "badvalue".into(),
                requires: vec![],
            }],
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
}
