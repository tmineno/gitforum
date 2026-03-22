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
///
/// The `on` field supports an optional kind prefix: `"dec:proposed->accepted"` restricts
/// the guard to DEC threads. Without a prefix (`"proposed->accepted"`), the guard applies
/// to all kinds that have the transition.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuardEntry {
    pub on: String,
    pub requires: Vec<GuardRule>,
    /// Parsed kind scope (None = applies to all kinds).
    #[serde(skip)]
    pub kind_scope: Option<ThreadKind>,
    /// Parsed transition string ("from->to"), without the kind prefix.
    #[serde(skip)]
    pub transition: String,
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
        let mut policy: Self = toml::from_str(&text)
            .map_err(|e| ForumError::Config(format!("invalid policy.toml: {e}")))?;
        policy.resolve_guard_scopes();
        Ok(policy)
    }

    /// Parse optional `kind:` prefix from each guard's `on` field and populate
    /// `kind_scope` and `transition`.
    pub fn resolve_guard_scopes(&mut self) {
        for guard in &mut self.guards {
            let (kind_scope, transition) = parse_guard_on(&guard.on);
            guard.kind_scope = kind_scope;
            guard.transition = transition;
        }
    }

    /// Return guards that apply to the given transition string for the given thread kind.
    ///
    /// Both kind-scoped guards matching `kind` and unscoped (wildcard) guards are returned.
    pub fn guards_for(&self, transition: &str, kind: ThreadKind) -> Vec<&GuardEntry> {
        self.guards
            .iter()
            .filter(|g| g.transition == transition && g.kind_scope.is_none_or(|k| k == kind))
            .collect()
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
    for guard in policy.guards_for(&transition, state.kind) {
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
        // Parse optional kind prefix.
        let (kind_scope, transition_part) = parse_guard_on(&guard.on);

        // Check for unknown kind prefix.
        if guard.on.contains(':') && kind_scope.is_none() {
            let prefix = guard.on.split_once(':').map(|(p, _)| p).unwrap_or("");
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard {:?}: unknown kind prefix {:?}; valid kinds are: issue, rfc, dec, task",
                    guard.on, prefix,
                ),
            });
        }

        if !transition_part.contains("->") {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "guard 'on' field {:?} is not a valid transition (expected 'from->to')",
                    guard.on
                ),
            });
            continue;
        }

        let parts: Vec<&str> = transition_part.splitn(2, "->").collect();
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

        // For kind-scoped guards, validate the transition against the specified kind.
        if let Some(scoped_kind) = kind_scope {
            if all_states.contains(from)
                && all_states.contains(to)
                && !state_machine::is_valid_transition(scoped_kind, from, to)
            {
                diags.push(LintDiag {
                    level: LintLevel::Warn,
                    message: format!(
                        "guard {:?}: not a valid transition for {}",
                        guard.on,
                        kind_name(&scoped_kind),
                    ),
                });
            }
            continue; // scoped guards don't need multi-kind check
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
                     consider using kind-scoped keys (e.g. \"issue:{}\") if you need different rules per kind",
                    guard.on,
                    matching_kinds.join(", "),
                    guard.on,
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

    // Semantic check: warn when an allow-list misses all non-terminal states for a kind.
    lint_allow_list_coverage(
        &mut diags,
        "revise_rules.allow_body_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_body_revise.as_slice())
            .unwrap_or(&[]),
        &all_kinds,
    );
    lint_allow_list_coverage(
        &mut diags,
        "revise_rules.allow_node_revise",
        policy
            .revise_rules
            .as_ref()
            .map(|r| r.allow_node_revise.as_slice())
            .unwrap_or(&[]),
        &all_kinds,
    );
    lint_allow_list_coverage(
        &mut diags,
        "evidence_rules.allow_evidence",
        policy
            .evidence_rules
            .as_ref()
            .map(|r| r.allow_evidence.as_slice())
            .unwrap_or(&[]),
        &all_kinds,
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

/// Terminal states are resolution/conclusion states where a thread's primary work is done.
const TERMINAL_STATES: &[&str] = &["closed", "rejected", "accepted", "deprecated"];

/// Return non-terminal states for a thread kind (states where active work happens).
fn non_terminal_states(kind: ThreadKind) -> Vec<&'static str> {
    let transitions = state_machine::valid_transitions(kind);
    let mut states: std::collections::BTreeSet<&str> = transitions
        .iter()
        .flat_map(|(from, to)| [*from, *to])
        .filter(|s| !TERMINAL_STATES.contains(s))
        .collect();
    // Include the initial state even if it only appears as a target
    let initial = initial_state(kind);
    states.insert(initial);
    states.into_iter().collect()
}

fn initial_state(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Issue | ThreadKind::Task => "open",
        ThreadKind::Rfc => "draft",
        ThreadKind::Dec => "proposed",
    }
}

/// Warn when an allow-list has no non-terminal states for an entire thread kind.
fn lint_allow_list_coverage(
    diags: &mut Vec<LintDiag>,
    field: &str,
    states: &[String],
    all_kinds: &[ThreadKind],
) {
    if states.is_empty() {
        return;
    }
    let state_set: std::collections::HashSet<&str> = states.iter().map(|s| s.as_str()).collect();

    for kind in all_kinds {
        let non_terminal = non_terminal_states(*kind);
        let has_any = non_terminal.iter().any(|s| state_set.contains(*s));
        if !has_any {
            diags.push(LintDiag {
                level: LintLevel::Warn,
                message: format!(
                    "{field}: no states for {} workflows; consider adding: {}",
                    kind_name(kind),
                    non_terminal.join(", "),
                ),
            });
        }
    }
}

/// Parse the `on` field of a guard entry into an optional kind scope and the transition string.
///
/// Formats:
/// - `"from->to"` → `(None, "from->to")`
/// - `"dec:from->to"` → `(Some(ThreadKind::Dec), "from->to")`
/// - Invalid kind prefix → `(None, original)` (lint will catch it)
fn parse_guard_on(on: &str) -> (Option<ThreadKind>, String) {
    if let Some((prefix, rest)) = on.split_once(':') {
        if let Some(kind) = parse_kind(prefix) {
            return (Some(kind), rest.to_string());
        }
    }
    (None, on.to_string())
}

fn parse_kind(s: &str) -> Option<ThreadKind> {
    match s {
        "issue" => Some(ThreadKind::Issue),
        "rfc" => Some(ThreadKind::Rfc),
        "dec" => Some(ThreadKind::Dec),
        "task" => Some(ThreadKind::Task),
        _ => None,
    }
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

    fn make_policy(guards: Vec<GuardEntry>) -> Policy {
        let mut p = Policy {
            guards,
            ..Default::default()
        };
        p.resolve_guard_scopes();
        p
    }

    fn minimal_policy() -> Policy {
        make_policy(vec![GuardEntry {
            on: "under-review->accepted".into(),
            requires: vec![
                GuardRule::NoOpenObjections,
                GuardRule::NoOpenActions,
                GuardRule::AtLeastOneSummary,
                GuardRule::OneHumanApproval,
            ],
            ..Default::default()
        }])
    }

    #[test]
    fn guards_for_matches_transition() {
        let policy = minimal_policy();
        assert_eq!(
            policy
                .guards_for("under-review->accepted", ThreadKind::Rfc)
                .len(),
            1
        );
        assert!(policy
            .guards_for("draft->under-review", ThreadKind::Rfc)
            .is_empty());
    }

    #[test]
    fn lint_valid_policy_returns_empty() {
        let policy = minimal_policy();
        assert!(lint_policy(&policy).is_empty());
    }

    #[test]
    fn lint_invalid_transition_reports_diag() {
        let policy = make_policy(vec![GuardEntry {
            on: "badvalue".into(),
            requires: vec![],
            ..Default::default()
        }]);
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
        let policy = make_policy(vec![GuardEntry {
            on: "bogus->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.level == LintLevel::Warn && d.message.contains("unknown state \"bogus\"")));
    }

    #[test]
    fn lint_unknown_guard_to_state() {
        let policy = make_policy(vec![GuardEntry {
            on: "open->fantasy".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unknown state \"fantasy\"")));
    }

    #[test]
    fn lint_notes_multi_kind_transition() {
        // "open->closed" applies to both issue and task
        let policy = make_policy(vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        let multi = diags
            .iter()
            .find(|d| d.message.contains("multiple thread kinds"));
        assert!(multi.is_some());
        assert_eq!(multi.unwrap().level, LintLevel::Note);
        assert!(multi.unwrap().message.contains("issue"));
        assert!(
            multi.unwrap().message.contains("kind-scoped keys"),
            "should suggest kind-scoped keys"
        );
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
        let policy = make_policy(vec![GuardEntry {
            on: "draft->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
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

    // ---- allow-list gap detection (ISSUE-0095) ----

    #[test]
    fn lint_warns_when_kind_missing_from_allow_list() {
        // Only RFC states — Issue, Dec, Task kinds are completely absent
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec!["draft".into(), "proposed".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message.contains("allow_body_revise")
            && d.message.contains("no states for issue")));
        assert!(diags.iter().any(|d| d.level == LintLevel::Warn
            && d.message.contains("allow_body_revise")
            && d.message.contains("no states for task")));
    }

    #[test]
    fn lint_no_gap_warning_when_kind_partially_covered() {
        // Has at least one non-terminal state per kind (even if not all)
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec![
                    "open".into(),     // issue + task
                    "draft".into(),    // rfc
                    "proposed".into(), // rfc + dec
                ],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let gap_warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn && d.message.contains("no states for"))
            .collect();
        assert!(
            gap_warnings.is_empty(),
            "unexpected gap warnings: {gap_warnings:?}"
        );
    }

    #[test]
    fn lint_no_gap_warning_for_empty_allow_list() {
        // Empty list = not configured, not a gap
        let policy = Policy {
            revise_rules: Some(ReviseRules {
                allow_body_revise: vec![],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let gap_warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("no states for"))
            .collect();
        assert!(
            gap_warnings.is_empty(),
            "empty list should not trigger gap warning"
        );
    }

    #[test]
    fn lint_gap_warning_includes_remediation_hint() {
        // Only task states — RFC kind is missing
        let policy = Policy {
            evidence_rules: Some(EvidenceRules {
                allow_evidence: vec![
                    "open".into(),
                    "designing".into(),
                    "implementing".into(),
                    "reviewing".into(),
                ],
            }),
            ..Default::default()
        };
        let diags = lint_policy(&policy);
        let rfc_gap = diags
            .iter()
            .find(|d| d.message.contains("no states for rfc"))
            .expect("should warn about missing RFC states");
        assert!(
            rfc_gap.message.contains("consider adding"),
            "should include remediation hint"
        );
        assert!(
            rfc_gap.message.contains("draft"),
            "should suggest RFC non-terminal states"
        );
    }

    // ---- kind-scoped guard keys (ISSUE-0097) ----

    #[test]
    fn guards_for_scoped_matches_only_specified_kind() {
        let policy = make_policy(vec![GuardEntry {
            on: "issue:open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        assert_eq!(
            policy.guards_for("open->closed", ThreadKind::Issue).len(),
            1
        );
        assert!(
            policy
                .guards_for("open->closed", ThreadKind::Task)
                .is_empty(),
            "scoped guard should not match other kinds"
        );
    }

    #[test]
    fn guards_for_unscoped_matches_all_kinds() {
        let policy = make_policy(vec![GuardEntry {
            on: "open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        assert_eq!(
            policy.guards_for("open->closed", ThreadKind::Issue).len(),
            1
        );
        assert_eq!(policy.guards_for("open->closed", ThreadKind::Task).len(), 1);
    }

    #[test]
    fn guards_for_union_of_scoped_and_unscoped() {
        let policy = make_policy(vec![
            GuardEntry {
                on: "open->closed".into(),
                requires: vec![GuardRule::NoOpenActions],
                ..Default::default()
            },
            GuardEntry {
                on: "issue:open->closed".into(),
                requires: vec![GuardRule::HasCommitEvidence],
                ..Default::default()
            },
        ]);
        // Issue gets both guards (union)
        assert_eq!(
            policy.guards_for("open->closed", ThreadKind::Issue).len(),
            2
        );
        // Task gets only the unscoped guard
        assert_eq!(policy.guards_for("open->closed", ThreadKind::Task).len(), 1);
    }

    #[test]
    fn lint_scoped_guard_valid() {
        let policy = make_policy(vec![GuardEntry {
            on: "dec:proposed->accepted".into(),
            requires: vec![GuardRule::NoOpenObjections],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.level == LintLevel::Warn)
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn lint_scoped_guard_unknown_kind() {
        let policy = make_policy(vec![GuardEntry {
            on: "boguskind:open->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Warn && d.message.contains("unknown kind prefix")),
            "should warn about unknown kind prefix: {diags:?}"
        );
    }

    #[test]
    fn lint_scoped_guard_invalid_transition_for_kind() {
        // "open->closed" is not a valid DEC transition
        let policy = make_policy(vec![GuardEntry {
            on: "dec:open->closed".into(),
            requires: vec![],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            diags.iter().any(|d| d.level == LintLevel::Warn
                && d.message.contains("not a valid transition for dec")),
            "should warn about invalid transition for kind: {diags:?}"
        );
    }

    #[test]
    fn lint_scoped_guard_skips_multi_kind_note() {
        // Scoped guard should not trigger multi-kind note
        let policy = make_policy(vec![GuardEntry {
            on: "issue:open->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }]);
        let diags = lint_policy(&policy);
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("multiple thread kinds")),
            "scoped guard should not trigger multi-kind note"
        );
    }

    #[test]
    fn parse_guard_on_unscoped() {
        let (kind, transition) = parse_guard_on("open->closed");
        assert!(kind.is_none());
        assert_eq!(transition, "open->closed");
    }

    #[test]
    fn parse_guard_on_scoped() {
        let (kind, transition) = parse_guard_on("dec:proposed->accepted");
        assert_eq!(kind, Some(ThreadKind::Dec));
        assert_eq!(transition, "proposed->accepted");
    }

    #[test]
    fn parse_guard_on_unknown_kind_prefix() {
        let (kind, transition) = parse_guard_on("bogus:open->closed");
        assert!(kind.is_none());
        assert_eq!(transition, "bogus:open->closed");
    }
}
