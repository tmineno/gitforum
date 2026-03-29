use super::event::{NodeType, ThreadKind};
use super::policy::Policy;

/// Severity of an operation check violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A violation detected by an operation check.
///
/// Preconditions: none.
/// Postconditions: describes one violation with rule name, reason, optional hint and fix command.
/// Failure modes: none (value type).
/// Side effects: none.
#[derive(Debug, Clone)]
pub struct OperationViolation {
    pub severity: Severity,
    pub rule: String,
    pub reason: String,
    pub hint: Option<String>,
    pub fix_command: Option<String>,
}

/// Check creation rules for a new thread.
///
/// Preconditions: `kind` is a valid ThreadKind; `body` is the thread body (may be None).
/// Postconditions: returns violations found; empty vec means all checks pass.
/// Failure modes: none (returns violations, not errors).
/// Side effects: none.
pub fn check_create(
    policy: &Policy,
    kind: ThreadKind,
    _title: &str,
    body: Option<&str>,
) -> Vec<OperationViolation> {
    let mut violations = Vec::new();
    let kind_key = kind.to_string();

    let Some(rules) = policy.creation_rules.get(&kind_key) else {
        return violations;
    };

    if rules.required_body && body.is_none_or(|b| b.trim().is_empty()) {
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "required_body".into(),
            reason: format!("{kind} threads require a body"),
            hint: Some("provide --body or --body-file".into()),
            fix_command: None,
        });
        // If body is entirely missing, skip section checks
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
                    // Check for empty section
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

/// Check whether a say (node) operation is allowed.
///
/// Preconditions: `status` is the current thread state; `node_type` is the type being added.
/// Postconditions: returns violations if the node type is not allowed in this state.
/// Failure modes: none.
/// Side effects: none.
pub fn check_say(policy: &Policy, status: &str, node_type: NodeType) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    if policy.node_rules.is_empty() {
        return violations;
    }

    if let Some(allowed) = policy.node_rules.get(status) {
        if !allowed.contains(&node_type) {
            violations.push(OperationViolation {
                severity: Severity::Error,
                rule: "node_type_restricted".into(),
                reason: format!("{node_type} nodes are not allowed in state '{status}'"),
                hint: if allowed.is_empty() {
                    Some(format!("no node types are allowed in state '{status}'"))
                } else {
                    let allowed_str: Vec<String> = allowed.iter().map(|n| n.to_string()).collect();
                    Some(format!("allowed in '{status}': {}", allowed_str.join(", ")))
                },
                fix_command: None,
            });
        }
    }
    // If the state is not listed in node_rules, all node types are allowed

    violations
}

/// Check whether a revise operation is allowed.
///
/// Preconditions: `status` is the current thread state; `is_body` indicates body vs node revision.
/// Postconditions: returns violations if revision is not allowed in this state.
/// Failure modes: none.
/// Side effects: none.
pub fn check_revise(policy: &Policy, status: &str, is_body: bool) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = &policy.revise_rules else {
        return violations;
    };

    let allowed = if is_body {
        &rules.allow_body_revise
    } else {
        &rules.allow_node_revise
    };

    // Empty list means no restrictions configured for this target
    if allowed.is_empty() {
        return violations;
    }

    if !allowed.iter().any(|s| s == status) {
        let target = if is_body { "body" } else { "node" };
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "revise_restricted".into(),
            reason: format!("{target} revision is not allowed in state '{status}'"),
            hint: Some(format!(
                "{target} revision is allowed in: {}",
                allowed.join(", ")
            )),
            fix_command: None,
        });
    }

    violations
}

/// Check whether adding evidence is allowed.
///
/// Preconditions: `status` is the current thread state.
/// Postconditions: returns violations if evidence is not allowed in this state.
/// Failure modes: none.
/// Side effects: none.
pub fn check_evidence(policy: &Policy, status: &str) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = &policy.evidence_rules else {
        return violations;
    };

    if rules.allow_evidence.is_empty() {
        return violations;
    }

    if !rules.allow_evidence.iter().any(|s| s == status) {
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "evidence_restricted".into(),
            reason: format!("evidence is not allowed in state '{status}'"),
            hint: Some(format!(
                "evidence is allowed in: {}",
                rules.allow_evidence.join(", ")
            )),
            fix_command: None,
        });
    }

    violations
}

/// Extract heading texts from markdown body.
/// Returns vec of (heading_text, is_empty) tuples.
/// A section is considered empty if there's no non-whitespace content between it and the next heading (or EOF).
fn extract_heading_texts(body: &str) -> Vec<(String, bool)> {
    let lines: Vec<&str> = body.lines().collect();
    let mut headings: Vec<(String, usize, usize)> = Vec::new(); // (text, line_idx, level)

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            // Count leading # characters and extract text
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
            // Find the next heading at the same or higher level (lower number)
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

/// Format operation violations for display on stderr.
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

/// Partition violations and determine whether to proceed.
/// Returns (has_errors, formatted_output).
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
            Severity::Error => Severity::Error, // --force never bypasses errors
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
    use std::collections::HashMap;

    fn policy_with_creation_rules() -> Policy {
        let mut creation_rules = HashMap::new();
        creation_rules.insert(
            "rfc".into(),
            super::super::policy::CreationRules {
                required_body: true,
                body_sections: vec!["Goal".into(), "Non-goals".into(), "Design".into()],
            },
        );
        creation_rules.insert(
            "issue".into(),
            super::super::policy::CreationRules {
                required_body: false,
                body_sections: vec![],
            },
        );
        Policy {
            creation_rules,
            ..Default::default()
        }
    }

    #[test]
    fn check_create_rfc_no_body_returns_error() {
        let policy = policy_with_creation_rules();
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", None);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(violations[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_empty_body_returns_error() {
        let policy = policy_with_creation_rules();
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", Some("  "));
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(violations[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_missing_sections_returns_warnings() {
        let policy = policy_with_creation_rules();
        let violations = check_create(
            &policy,
            ThreadKind::Rfc,
            "Test",
            Some("## Goal\nSome goal text"),
        );
        // Missing Non-goals and Design
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().all(|v| v.severity == Severity::Warning));
        assert!(violations.iter().any(|v| v.reason.contains("Non-goals")));
        assert!(violations.iter().any(|v| v.reason.contains("Design")));
    }

    #[test]
    fn check_create_rfc_all_sections_present() {
        let policy = policy_with_creation_rules();
        let body = "## Goal\ntext\n## Non-goals\ntext\n## Design\ntext";
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_rfc_empty_section_returns_warning() {
        let policy = policy_with_creation_rules();
        let body = "## Goal\n\n## Non-goals\ntext\n## Design\ntext";
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Warning);
        assert_eq!(violations[0].rule, "body_section_empty");
    }

    #[test]
    fn check_create_rfc_section_with_subheadings_not_empty() {
        let policy = policy_with_creation_rules();
        let body =
            "## Goal\ntext\n## Non-goals\ntext\n## Design\n\n### Option A\nDetails\n\n### Option B\nMore";
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
        assert!(
            violations.is_empty(),
            "sub-headings should count as content: {violations:?}"
        );
    }

    #[test]
    fn check_create_issue_no_body_allowed() {
        let policy = policy_with_creation_rules();
        let violations = check_create(&policy, ThreadKind::Issue, "Bug", None);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_no_policy_allows_everything() {
        let policy = Policy::default();
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", None);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_case_insensitive_heading_match() {
        let policy = policy_with_creation_rules();
        let body = "# goal\ntext\n### NON-GOALS\ntext\n## design\ntext";
        let violations = check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
        assert!(violations.is_empty());
    }

    #[test]
    fn check_say_allowed() {
        let mut node_rules = HashMap::new();
        node_rules.insert(
            "draft".into(),
            vec![NodeType::Claim, NodeType::Question, NodeType::Objection],
        );
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        let violations = check_say(&policy, "draft", NodeType::Claim);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_say_blocked() {
        let mut node_rules = HashMap::new();
        node_rules.insert("accepted".into(), vec![]);
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        let violations = check_say(&policy, "accepted", NodeType::Claim);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(violations[0].rule, "node_type_restricted");
    }

    #[test]
    fn check_say_unlisted_state_allows_all() {
        let mut node_rules = HashMap::new();
        node_rules.insert("accepted".into(), vec![]);
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        // "draft" not listed in node_rules → all allowed
        let violations = check_say(&policy, "draft", NodeType::Claim);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_say_no_policy_allows_all() {
        let policy = Policy::default();
        let violations = check_say(&policy, "accepted", NodeType::Claim);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_revise_body_allowed() {
        let policy = Policy {
            revise_rules: Some(super::super::policy::ReviseRules {
                allow_body_revise: vec!["draft".into(), "proposed".into()],
                allow_node_revise: vec!["draft".into()],
            }),
            ..Default::default()
        };
        let violations = check_revise(&policy, "draft", true);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_revise_body_blocked() {
        let policy = Policy {
            revise_rules: Some(super::super::policy::ReviseRules {
                allow_body_revise: vec!["draft".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let violations = check_revise(&policy, "accepted", true);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
    }

    #[test]
    fn check_revise_no_policy_allows_all() {
        let policy = Policy::default();
        let violations = check_revise(&policy, "accepted", true);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_evidence_allowed() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec!["draft".into(), "open".into()],
            }),
            ..Default::default()
        };
        let violations = check_evidence(&policy, "draft");
        assert!(violations.is_empty());
    }

    #[test]
    fn check_evidence_blocked() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec!["draft".into()],
            }),
            ..Default::default()
        };
        let violations = check_evidence(&policy, "accepted");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
    }

    #[test]
    fn check_evidence_no_policy_allows_all() {
        let policy = Policy::default();
        let violations = check_evidence(&policy, "accepted");
        assert!(violations.is_empty());
    }

    #[test]
    fn evaluate_violations_no_violations() {
        let (has_errors, output) = evaluate_violations(&[], false, false);
        assert!(!has_errors);
        assert!(output.is_empty());
    }

    #[test]
    fn evaluate_violations_error_blocks() {
        let violations = vec![OperationViolation {
            severity: Severity::Error,
            rule: "test".into(),
            reason: "blocked".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&violations, false, false);
        assert!(has_errors);
    }

    #[test]
    fn evaluate_violations_warning_does_not_block() {
        let violations = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&violations, false, false);
        assert!(!has_errors);
    }

    #[test]
    fn evaluate_violations_strict_warning_blocks() {
        let violations = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&violations, false, true);
        assert!(has_errors);
    }

    #[test]
    fn evaluate_violations_strict_force_warning_passes() {
        let violations = vec![OperationViolation {
            severity: Severity::Warning,
            rule: "test".into(),
            reason: "advisory".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&violations, true, true);
        assert!(!has_errors);
    }

    #[test]
    fn evaluate_violations_force_does_not_bypass_error() {
        let violations = vec![OperationViolation {
            severity: Severity::Error,
            rule: "test".into(),
            reason: "blocked".into(),
            hint: None,
            fix_command: None,
        }];
        let (has_errors, _) = evaluate_violations(&violations, true, false);
        assert!(has_errors);
    }

    #[test]
    fn extract_headings_various_levels() {
        let body = "# Goal\ntext\n## Non-goals\ntext\n### Design\ntext";
        let headings = extract_heading_texts(body);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].0, "Goal");
        assert_eq!(headings[1].0, "Non-goals");
        assert_eq!(headings[2].0, "Design");
        assert!(headings.iter().all(|(_, empty)| !empty));
    }

    #[test]
    fn extract_headings_empty_section() {
        let body = "## Goal\n\n## Design\ntext";
        let headings = extract_heading_texts(body);
        assert_eq!(headings.len(), 2);
        assert!(headings[0].1); // Goal is empty
        assert!(!headings[1].1); // Design has content
    }
}
