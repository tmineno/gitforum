//! SPEC-3.0 §3.3 operation checks: per-category creation / allowed-node-
//! type / revise / evidence rules.
//!
//! 3.0 operation checks are category-keyed (no lifecycle / facet
//! selectors). Callers pass the thread's `category` (`"rfc"` or
//! `"task"`) along with the relevant status / node kind for dispatch.

use super::node::NodeKind;
use super::policy::{normalize_state_name, Policy};

/// State-name allow-list match that tolerates 1.x↔2.0 name mismatches
/// inherited from migrated chains. State stored in 3.0 snapshots is
/// always 2.0-canonical, but the helper preserves migration tolerance
/// for legacy fixtures.
fn allow_list_contains(allow: &[String], status: &str) -> bool {
    let target = normalize_state_name(status);
    allow
        .iter()
        .any(|s| normalize_state_name(s.as_str()) == target)
}

fn render_allow_list_for_hint(allow: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for entry in allow {
        let canonical = normalize_state_name(entry.as_str());
        if !seen.contains(&canonical) {
            seen.push(canonical);
        }
    }
    seen.join(", ")
}

fn node_kind_str(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Comment => "comment",
        NodeKind::Approval => "approval",
        NodeKind::Objection => "objection",
        NodeKind::Action => "action",
    }
}

/// SPEC-3.0 §3.3 — table-driven dispatch over the four operation check
/// kinds. Each variant carries the category-scoped context the
/// corresponding rule lookup needs.
#[derive(Debug)]
pub enum Op<'a> {
    /// New thread under creation. Resolution: `categories.<C>.creation`.
    Create {
        category: &'a str,
        body: Option<&'a str>,
    },
    /// Adding a node to an existing thread.
    Say {
        category: &'a str,
        status: &'a str,
        node_type: NodeKind,
    },
    /// Revising the body or a node body of an existing thread.
    Revise {
        category: &'a str,
        status: &'a str,
        is_body: bool,
    },
    /// Attaching evidence to an existing thread.
    Evidence { category: &'a str, status: &'a str },
}

pub fn check_op(policy: &Policy, op: Op<'_>) -> Vec<OperationViolation> {
    match op {
        Op::Create { category, body } => check_create_inner(policy, category, body),
        Op::Say {
            category,
            status,
            node_type,
        } => check_say_inner(policy, category, status, node_type),
        Op::Revise {
            category,
            status,
            is_body,
        } => check_revise_inner(policy, category, status, is_body),
        Op::Evidence { category, status } => check_evidence_inner(policy, category, status),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct OperationViolation {
    pub severity: Severity,
    pub rule: String,
    pub reason: String,
    pub hint: Option<String>,
    pub fix_command: Option<String>,
}

pub fn check_create(
    policy: &Policy,
    category: &str,
    _title: &str,
    body: Option<&str>,
) -> Vec<OperationViolation> {
    check_create_inner(policy, category, body)
}

fn check_create_inner(
    policy: &Policy,
    category: &str,
    body: Option<&str>,
) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = policy.creation_rules_for(category) else {
        return violations;
    };

    if rules.required_body && body.is_none_or(|b| b.trim().is_empty()) {
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "required_body".into(),
            reason: format!("category `{category}` threads require a body"),
            hint: Some("provide --body or --body-file".into()),
            fix_command: None,
        });
        return violations;
    }

    if !rules.body_sections.is_empty() {
        if let Some(body_text) = body {
            let found_sections = extract_heading_texts(body_text);
            for required in &rules.body_sections {
                let required_lower = required.to_lowercase();
                let section_present = found_sections
                    .iter()
                    .any(|(text, _)| text.to_lowercase() == required_lower);
                if !section_present {
                    violations.push(OperationViolation {
                        severity: Severity::Warning,
                        rule: "body_section".into(),
                        reason: format!("missing required section: {required}"),
                        hint: Some(format!("add a heading: ## {required}")),
                        fix_command: None,
                    });
                } else {
                    let is_empty = found_sections
                        .iter()
                        .any(|(text, empty)| text.to_lowercase() == required_lower && *empty);
                    if is_empty {
                        violations.push(OperationViolation {
                            severity: Severity::Warning,
                            rule: "body_section_empty".into(),
                            reason: format!("section is empty: {required}"),
                            hint: Some(format!("add content under the {required} heading")),
                            fix_command: None,
                        });
                    }
                }
            }
        }
    }

    violations
}

pub fn check_say(
    policy: &Policy,
    category: &str,
    status: &str,
    node_type: NodeKind,
) -> Vec<OperationViolation> {
    check_say_inner(policy, category, status, node_type)
}

fn check_say_inner(
    policy: &Policy,
    category: &str,
    status: &str,
    node_type: NodeKind,
) -> Vec<OperationViolation> {
    let mut violations = Vec::new();
    let target_kind = node_type;
    let target_status = normalize_state_name(status);

    // 3.0 lookup is direct on the canonical status name; allow_list_contains
    // remains in case migrated fixtures still carry 1.x state names.
    let category_policy = policy.category(category);
    let Some(cat) = category_policy else {
        return violations;
    };

    // Find an allowed_node_types entry whose key normalizes to target.
    let entry = cat
        .allowed_node_types
        .iter()
        .find(|(k, _)| normalize_state_name(k.as_str()) == target_status);
    if let Some((_, allowed)) = entry {
        if !allowed.contains(&target_kind) {
            violations.push(OperationViolation {
                severity: Severity::Error,
                rule: "node_type_restricted".into(),
                reason: format!(
                    "{} nodes are not allowed in state '{target_status}'",
                    node_kind_str(target_kind)
                ),
                hint: if allowed.is_empty() {
                    Some(format!(
                        "no node types are allowed in state '{target_status}'"
                    ))
                } else {
                    let allowed_str: Vec<String> = allowed
                        .iter()
                        .map(|k| node_kind_str(*k).to_string())
                        .collect();
                    Some(format!(
                        "allowed in '{target_status}': {}",
                        allowed_str.join(", ")
                    ))
                },
                fix_command: None,
            });
        }
    }
    // If the status is not listed, all node types are allowed.

    violations
}

pub fn check_revise(
    policy: &Policy,
    category: &str,
    status: &str,
    is_body: bool,
) -> Vec<OperationViolation> {
    check_revise_inner(policy, category, status, is_body)
}

fn check_revise_inner(
    policy: &Policy,
    category: &str,
    status: &str,
    is_body: bool,
) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = policy.revise_rules_for(category) else {
        return violations;
    };

    // SPEC-3.0 §3.3: an absent allow list (None) means no restriction;
    // a present-but-empty list (Some(vec![])) is an explicit deny-all.
    let allowed_opt = if is_body {
        rules.allow_body_revise.as_ref()
    } else {
        rules.allow_node_revise.as_ref()
    };
    let Some(allowed) = allowed_opt else {
        return violations;
    };

    if allow_list_contains(allowed, status) {
        return violations;
    }

    let target = if is_body { "body" } else { "node" };
    let canonical_status = normalize_state_name(status);
    let hint = if allowed.is_empty() {
        format!("{target} revision is denied in every status (allow list is empty)")
    } else {
        format!(
            "{target} revision is allowed in: {}",
            render_allow_list_for_hint(allowed)
        )
    };
    violations.push(OperationViolation {
        severity: Severity::Error,
        rule: "revise_restricted".into(),
        reason: format!("{target} revision is not allowed in state '{canonical_status}'"),
        hint: Some(hint),
        fix_command: None,
    });

    violations
}

pub fn check_evidence(policy: &Policy, category: &str, status: &str) -> Vec<OperationViolation> {
    check_evidence_inner(policy, category, status)
}

fn check_evidence_inner(policy: &Policy, category: &str, status: &str) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = policy.evidence_rules_for(category) else {
        return violations;
    };

    // SPEC-3.0 §3.3: present-but-empty `allow_evidence = []` denies
    // attachment in every status; absent section means no restriction.
    let Some(allowed) = rules.allow_evidence.as_ref() else {
        return violations;
    };

    if allow_list_contains(allowed, status) {
        return violations;
    }

    let canonical_status = normalize_state_name(status);
    let hint = if allowed.is_empty() {
        "evidence is denied in every status (allow list is empty)".to_string()
    } else {
        format!(
            "evidence is allowed in: {}",
            render_allow_list_for_hint(allowed)
        )
    };
    violations.push(OperationViolation {
        severity: Severity::Error,
        rule: "evidence_restricted".into(),
        reason: format!("evidence is not allowed in state '{canonical_status}'"),
        hint: Some(hint),
        fix_command: None,
    });

    violations
}

fn extract_heading_texts(body: &str) -> Vec<(String, bool)> {
    let lines: Vec<&str> = body.lines().collect();
    let mut headings: Vec<(String, usize, usize)> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let mut hashes = 1;
            let mut chars = rest.chars();
            while chars.as_str().starts_with('#') {
                hashes += 1;
                chars.next();
            }
            let heading_text = chars.as_str().trim().to_string();
            if !heading_text.is_empty() {
                headings.push((heading_text, i, hashes));
            }
        }
    }

    headings
        .iter()
        .enumerate()
        .map(|(idx, (text, line_idx, level))| {
            let next_heading_line = headings[idx + 1..]
                .iter()
                .find(|(_, _, l)| *l <= *level)
                .map(|(_, li, _)| *li)
                .unwrap_or(lines.len());
            let content_between = lines[line_idx + 1..next_heading_line]
                .iter()
                .any(|l| !l.trim().is_empty());
            (text.clone(), !content_between)
        })
        .collect()
}

pub fn format_violations(violations: &[OperationViolation]) -> String {
    let mut out = String::new();
    for v in violations {
        let severity_tag = match v.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        out.push_str(&format!("{severity_tag}: [{}] {}\n", v.rule, v.reason));
        if let Some(hint) = &v.hint {
            out.push_str(&format!("  hint: {hint}\n"));
        }
        if let Some(fix) = &v.fix_command {
            out.push_str(&format!("  fix: {fix}\n"));
        }
    }
    out
}

pub fn evaluate_violations(
    violations: &[OperationViolation],
    force: bool,
    strict: bool,
) -> (bool, String) {
    if violations.is_empty() {
        return (false, String::new());
    }

    let mut effective_errors = false;
    let mut out = String::new();

    for v in violations {
        let effective_severity = match v.severity {
            Severity::Error => Severity::Error,
            Severity::Warning if strict && !force => Severity::Error,
            Severity::Warning => Severity::Warning,
        };

        let severity_tag = match effective_severity {
            Severity::Error => {
                effective_errors = true;
                "error"
            }
            Severity::Warning => "warning",
        };

        out.push_str(&format!("{severity_tag}: [{}] {}\n", v.rule, v.reason));
        if let Some(hint) = &v.hint {
            out.push_str(&format!("  hint: {hint}\n"));
        }
        if let Some(fix) = &v.fix_command {
            out.push_str(&format!("  fix: {fix}\n"));
        }
    }

    (effective_errors, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::policy::{CategoryPolicy, CreationRules, EvidenceRules, ReviseRules};
    use std::collections::HashMap;

    fn policy_with_creation_rules() -> Policy {
        let mut policy = Policy::default();
        let rfc = CategoryPolicy {
            creation: Some(CreationRules {
                required_body: true,
                body_sections: vec!["Goal".into(), "Non-goals".into(), "Design".into()],
            }),
            ..CategoryPolicy::default()
        };
        policy.categories.insert("rfc".into(), rfc);
        let task = CategoryPolicy {
            creation: Some(CreationRules {
                required_body: false,
                body_sections: vec![],
            }),
            ..CategoryPolicy::default()
        };
        policy.categories.insert("task".into(), task);
        policy
    }

    #[test]
    fn check_create_rfc_no_body_returns_error() {
        let policy = policy_with_creation_rules();
        let v = check_create(&policy, "rfc", "Test", None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].severity, Severity::Error);
        assert_eq!(v[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_empty_body_returns_error() {
        let policy = policy_with_creation_rules();
        let v = check_create(&policy, "rfc", "Test", Some("  "));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_missing_sections_returns_warnings() {
        let policy = policy_with_creation_rules();
        let v = check_create(&policy, "rfc", "Test", Some("## Goal\nSome goal text"));
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| x.severity == Severity::Warning));
        assert!(v.iter().any(|x| x.reason.contains("Non-goals")));
        assert!(v.iter().any(|x| x.reason.contains("Design")));
    }

    #[test]
    fn check_create_rfc_all_sections_present() {
        let policy = policy_with_creation_rules();
        let body = "## Goal\ntext\n## Non-goals\ntext\n## Design\ntext";
        let v = check_create(&policy, "rfc", "Test", Some(body));
        assert!(v.is_empty());
    }

    #[test]
    fn check_create_rfc_empty_section_returns_warning() {
        let policy = policy_with_creation_rules();
        let body = "## Goal\n\n## Non-goals\ntext\n## Design\ntext";
        let v = check_create(&policy, "rfc", "Test", Some(body));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "body_section_empty");
    }

    #[test]
    fn check_create_task_no_body_allowed() {
        let policy = policy_with_creation_rules();
        let v = check_create(&policy, "task", "Bug", None);
        assert!(v.is_empty());
    }

    #[test]
    fn check_create_no_policy_allows_everything() {
        let policy = Policy::default();
        let v = check_create(&policy, "rfc", "Test", None);
        assert!(v.is_empty());
    }

    #[test]
    fn check_create_unknown_category_allows_everything() {
        let policy = policy_with_creation_rules();
        let v = check_create(&policy, "bogus", "Test", None);
        assert!(v.is_empty());
    }

    fn policy_with_node_rules(category: &str, status: &str, kinds: Vec<NodeKind>) -> Policy {
        let mut allowed = HashMap::new();
        allowed.insert(status.to_string(), kinds);
        let cat = CategoryPolicy {
            allowed_node_types: allowed,
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert(category.to_string(), cat);
        policy
    }

    #[test]
    fn check_say_allowed() {
        let policy = policy_with_node_rules(
            "rfc",
            "draft",
            vec![NodeKind::Comment, NodeKind::Objection, NodeKind::Action],
        );
        let v = check_say(&policy, "rfc", "draft", NodeKind::Comment);
        assert!(v.is_empty());
    }

    #[test]
    fn check_say_blocked() {
        let policy = policy_with_node_rules("rfc", "done", vec![]);
        let v = check_say(&policy, "rfc", "done", NodeKind::Comment);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "node_type_restricted");
    }

    #[test]
    fn check_say_unlisted_state_allows_all() {
        let policy = policy_with_node_rules("rfc", "done", vec![]);
        // "draft" not listed → all allowed.
        let v = check_say(&policy, "rfc", "draft", NodeKind::Comment);
        assert!(v.is_empty());
    }

    #[test]
    fn check_say_no_policy_allows_all() {
        let policy = Policy::default();
        let v = check_say(&policy, "rfc", "done", NodeKind::Comment);
        assert!(v.is_empty());
    }

    fn policy_with_revise(
        category: &str,
        body_states: Vec<&str>,
        node_states: Vec<&str>,
    ) -> Policy {
        let cat = CategoryPolicy {
            revise: Some(ReviseRules {
                allow_body_revise: Some(body_states.into_iter().map(String::from).collect()),
                allow_node_revise: Some(node_states.into_iter().map(String::from).collect()),
            }),
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert(category.to_string(), cat);
        policy
    }

    #[test]
    fn check_revise_body_allowed() {
        let policy = policy_with_revise("rfc", vec!["draft", "open"], vec!["draft"]);
        let v = check_revise(&policy, "rfc", "draft", true);
        assert!(v.is_empty());
    }

    #[test]
    fn check_revise_body_blocked() {
        let policy = policy_with_revise("rfc", vec!["draft"], vec![]);
        let v = check_revise(&policy, "rfc", "done", true);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "revise_restricted");
    }

    #[test]
    fn check_revise_no_policy_allows_all() {
        let policy = Policy::default();
        let v = check_revise(&policy, "rfc", "done", true);
        assert!(v.is_empty());
    }

    #[test]
    fn check_revise_absent_section_allows_all() {
        // SPEC-3.0 §3.3: an absent revise section means no restriction.
        // The category is present in the policy with other rules but no
        // `revise` table at all.
        let mut policy = Policy::default();
        policy
            .categories
            .insert("rfc".into(), CategoryPolicy::default());
        assert!(check_revise(&policy, "rfc", "done", true).is_empty());
        assert!(check_revise(&policy, "rfc", "done", false).is_empty());
    }

    #[test]
    fn check_revise_present_but_empty_body_denies_all() {
        // SPEC-3.0 §3.3: present-but-empty `allow_body_revise = []` is an
        // explicit deny-all; absent is the only "no restriction" form.
        let cat = CategoryPolicy {
            revise: Some(ReviseRules {
                allow_body_revise: Some(vec![]),
                allow_node_revise: None,
            }),
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert("rfc".into(), cat);
        for status in ["draft", "open", "review", "done"] {
            let v = check_revise(&policy, "rfc", status, true);
            assert_eq!(v.len(), 1, "status {status}: expected 1 violation");
            assert_eq!(v[0].rule, "revise_restricted");
            // Node revision still unrestricted (None).
            assert!(check_revise(&policy, "rfc", status, false).is_empty());
        }
    }

    #[test]
    fn check_revise_present_but_empty_node_denies_all() {
        let cat = CategoryPolicy {
            revise: Some(ReviseRules {
                allow_body_revise: None,
                allow_node_revise: Some(vec![]),
            }),
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert("rfc".into(), cat);
        for status in ["draft", "open", "review", "done"] {
            let v = check_revise(&policy, "rfc", status, false);
            assert_eq!(v.len(), 1, "status {status}: expected 1 violation");
            // Body revision still unrestricted (None).
            assert!(check_revise(&policy, "rfc", status, true).is_empty());
        }
    }

    fn policy_with_evidence(category: &str, states: Vec<&str>) -> Policy {
        let cat = CategoryPolicy {
            evidence: Some(EvidenceRules {
                allow_evidence: Some(states.into_iter().map(String::from).collect()),
            }),
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert(category.to_string(), cat);
        policy
    }

    #[test]
    fn check_evidence_allowed() {
        let policy = policy_with_evidence("rfc", vec!["draft", "open"]);
        let v = check_evidence(&policy, "rfc", "draft");
        assert!(v.is_empty());
    }

    #[test]
    fn check_evidence_blocked() {
        let policy = policy_with_evidence("rfc", vec!["draft"]);
        let v = check_evidence(&policy, "rfc", "done");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "evidence_restricted");
    }

    #[test]
    fn check_evidence_no_policy_allows_all() {
        let policy = Policy::default();
        let v = check_evidence(&policy, "rfc", "done");
        assert!(v.is_empty());
    }

    #[test]
    fn check_evidence_absent_section_allows_all() {
        let mut policy = Policy::default();
        policy
            .categories
            .insert("rfc".into(), CategoryPolicy::default());
        assert!(check_evidence(&policy, "rfc", "done").is_empty());
    }

    #[test]
    fn check_evidence_present_but_empty_denies_all() {
        // SPEC-3.0 §3.3: present-but-empty `allow_evidence = []` denies
        // attachment in every status.
        let cat = CategoryPolicy {
            evidence: Some(EvidenceRules {
                allow_evidence: Some(vec![]),
            }),
            ..CategoryPolicy::default()
        };
        let mut policy = Policy::default();
        policy.categories.insert("rfc".into(), cat);
        for status in ["draft", "open", "review", "done"] {
            let v = check_evidence(&policy, "rfc", status);
            assert_eq!(v.len(), 1, "status {status}: expected 1 violation");
            assert_eq!(v[0].rule, "evidence_restricted");
        }
    }

    #[test]
    fn evaluate_violations_no_violations() {
        let (has_errors, output) = evaluate_violations(&[], false, false);
        assert!(!has_errors);
        assert!(output.is_empty());
    }

    #[test]
    fn evaluate_violations_error_blocks() {
        let v = vec![OperationViolation {
            severity: Severity::Error,
            rule: "test".into(),
            reason: "blocked".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&v, false, false);
        assert!(has_errors);
    }

    #[test]
    fn evaluate_violations_warning_does_not_block() {
        let v = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&v, false, false);
        assert!(!has_errors);
    }

    #[test]
    fn evaluate_violations_strict_warning_blocks() {
        let v = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&v, false, true);
        assert!(has_errors);
    }

    #[test]
    fn evaluate_violations_strict_force_warning_passes() {
        let v = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&v, true, true);
        assert!(!has_errors);
    }

    #[test]
    fn evaluate_violations_force_does_not_bypass_error() {
        let v = vec![OperationViolation {
            severity: Severity::Error,
            rule: "test".into(),
            reason: "blocked".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&v, true, false);
        assert!(has_errors);
    }

    #[test]
    fn check_op_create_dispatches_to_creation_rules() {
        let policy = policy_with_creation_rules();
        let v = check_op(
            &policy,
            Op::Create {
                category: "rfc",
                body: None,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "required_body");
    }

    #[test]
    fn check_op_say_dispatches_to_node_rules() {
        let policy = policy_with_node_rules("rfc", "done", vec![]);
        let v = check_op(
            &policy,
            Op::Say {
                category: "rfc",
                status: "done",
                node_type: NodeKind::Comment,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "node_type_restricted");
    }

    #[test]
    fn check_op_revise_dispatches_to_revise_rules() {
        let policy = policy_with_revise("rfc", vec!["draft"], vec![]);
        let v = check_op(
            &policy,
            Op::Revise {
                category: "rfc",
                status: "done",
                is_body: true,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "revise_restricted");
    }

    #[test]
    fn check_op_evidence_dispatches_to_evidence_rules() {
        let policy = policy_with_evidence("rfc", vec!["draft"]);
        let v = check_op(
            &policy,
            Op::Evidence {
                category: "rfc",
                status: "done",
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "evidence_restricted");
    }
}
