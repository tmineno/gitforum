use super::event::{normalize_state_name, Lifecycle, NodeType};
use super::policy::Policy;

/// State-name allow-list match that tolerates 1.x↔2.0 name mismatches.
///
/// State events store 2.0 canonical names (per `state_change`), but
/// user policies may still carry 1.x names (`under-review`, `reviewing`,
/// `closed`, `accepted`, `implementing`, `designing`, `pending`).
/// Normalizing both sides via `normalize_state_name` lets either form
/// in the policy match either form on the live thread.
fn allow_list_contains(allow: &[String], status: &str) -> bool {
    let target = normalize_state_name(status);
    allow
        .iter()
        .any(|s| normalize_state_name(s.as_str()) == target)
}

/// Render a policy allow-list for human-readable error hints, using 2.0
/// canonical state names with deduplication. Without this, hints can
/// echo a legacy-only allow-list and read as "evidence not allowed in
/// state 'review'; allowed in: ..., reviewing, ..." — which lists the
/// user's current state under a different name and reads as
/// self-contradictory.
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

/// SPEC-2.0 §7.2 / RFC-nm3d31yk Track D — table-driven dispatch over the
/// four operation check kinds. Each variant carries the context the
/// corresponding rule-table lookup needs.
#[derive(Debug)]
pub enum Op<'a> {
    /// New thread under creation. Resolution is most-specific-match
    /// over `creation_rules.<lifecycle>` ± `.tag.<name>` (SPEC-2.0 §7.2).
    Create {
        lifecycle: Lifecycle,
        tags: &'a [String],
        body: Option<&'a str>,
    },
    /// Adding a node (`say`) to an existing thread.
    Say {
        status: &'a str,
        node_type: NodeType,
    },
    /// Revising the body or a node body of an existing thread.
    Revise { status: &'a str, is_body: bool },
    /// Attaching evidence to an existing thread.
    Evidence { status: &'a str },
}

/// SPEC-2.0 §7.2 unified entry point: one operation check function
/// dispatches to the rule-table lookup for the requested op kind.
///
/// Preconditions: `policy` is loaded; `op` carries the operation
/// context (lifecycle / tags / status / etc. depending on the variant).
/// Postconditions: returns the list of violations the per-op rule
/// table produced; an empty vec means all checks pass.
pub fn check_op(policy: &Policy, op: Op<'_>) -> Vec<OperationViolation> {
    match op {
        Op::Create {
            lifecycle,
            tags,
            body,
        } => check_create_inner(policy, lifecycle, tags, body),
        Op::Say { status, node_type } => check_say_inner(policy, status, node_type),
        Op::Revise { status, is_body } => check_revise_inner(policy, status, is_body),
        Op::Evidence { status } => check_evidence_inner(policy, status),
    }
}

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

/// Backward-compatibility shim. Prefer `check_op(Op::Create { .. })`
/// for new call sites; this thin wrapper exists so legacy callers keep
/// compiling during the §7.2 dispatch transition.
pub fn check_create(
    policy: &Policy,
    lifecycle: Lifecycle,
    tags: &[String],
    _title: &str,
    body: Option<&str>,
) -> Vec<OperationViolation> {
    check_create_inner(policy, lifecycle, tags, body)
}

fn check_create_inner(
    policy: &Policy,
    lifecycle: Lifecycle,
    tags: &[String],
    body: Option<&str>,
) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = policy.resolve_creation_rules(lifecycle, tags) else {
        return violations;
    };

    if rules.required_body && body.is_none_or(|b| b.trim().is_empty()) {
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "required_body".into(),
            reason: format!("{lifecycle} threads require a body"),
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

/// Backward-compatibility shim — see `check_op`.
pub fn check_say(policy: &Policy, status: &str, node_type: NodeType) -> Vec<OperationViolation> {
    check_say_inner(policy, status, node_type)
}

fn check_say_inner(policy: &Policy, status: &str, node_type: NodeType) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    if policy.node_rules.is_empty() {
        return violations;
    }

    let target = normalize_state_name(status);
    let matched = policy
        .node_rules
        .iter()
        .find(|(key, _)| normalize_state_name(key.as_str()) == target);
    if let Some((_, allowed)) = matched {
        if !allowed.contains(&node_type) {
            violations.push(OperationViolation {
                severity: Severity::Error,
                rule: "node_type_restricted".into(),
                reason: format!("{node_type} nodes are not allowed in state '{target}'"),
                hint: if allowed.is_empty() {
                    Some(format!("no node types are allowed in state '{target}'"))
                } else {
                    let allowed_str: Vec<String> = allowed.iter().map(|n| n.to_string()).collect();
                    Some(format!("allowed in '{target}': {}", allowed_str.join(", ")))
                },
                fix_command: None,
            });
        }
    }
    // If the state is not listed in node_rules, all node types are allowed

    violations
}

/// Backward-compatibility shim — see `check_op`.
pub fn check_revise(policy: &Policy, status: &str, is_body: bool) -> Vec<OperationViolation> {
    check_revise_inner(policy, status, is_body)
}

fn check_revise_inner(policy: &Policy, status: &str, is_body: bool) -> Vec<OperationViolation> {
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

    if !allow_list_contains(allowed, status) {
        let target = if is_body { "body" } else { "node" };
        let canonical_status = normalize_state_name(status);
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "revise_restricted".into(),
            reason: format!("{target} revision is not allowed in state '{canonical_status}'"),
            hint: Some(format!(
                "{target} revision is allowed in: {}",
                render_allow_list_for_hint(allowed)
            )),
            fix_command: None,
        });
    }

    violations
}

/// Backward-compatibility shim — see `check_op`.
pub fn check_evidence(policy: &Policy, status: &str) -> Vec<OperationViolation> {
    check_evidence_inner(policy, status)
}

fn check_evidence_inner(policy: &Policy, status: &str) -> Vec<OperationViolation> {
    let mut violations = Vec::new();

    let Some(rules) = &policy.evidence_rules else {
        return violations;
    };

    if rules.allow_evidence.is_empty() {
        return violations;
    }

    if !allow_list_contains(&rules.allow_evidence, status) {
        let canonical_status = normalize_state_name(status);
        violations.push(OperationViolation {
            severity: Severity::Error,
            rule: "evidence_restricted".into(),
            reason: format!("evidence is not allowed in state '{canonical_status}'"),
            hint: Some(format!(
                "evidence is allowed in: {}",
                render_allow_list_for_hint(&rules.allow_evidence)
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
        use super::super::policy::{CreationRules, LifecycleCreationRules};
        let mut creation_rules = HashMap::new();
        // SPEC-2.0 §7.2: rules keyed by lifecycle, with optional tag overlays.
        creation_rules.insert(
            "proposal".into(),
            LifecycleCreationRules {
                base: CreationRules {
                    required_body: true,
                    body_sections: vec!["Goal".into(), "Non-goals".into(), "Design".into()],
                },
                tag: HashMap::new(),
            },
        );
        creation_rules.insert(
            "execution".into(),
            LifecycleCreationRules {
                base: CreationRules {
                    required_body: false,
                    body_sections: vec![],
                },
                tag: HashMap::new(),
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
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            None,
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(violations[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_empty_body_returns_error() {
        let policy = policy_with_creation_rules();
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            Some("  "),
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(violations[0].rule, "required_body");
    }

    #[test]
    fn check_create_rfc_missing_sections_returns_warnings() {
        let policy = policy_with_creation_rules();
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
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
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            Some(body),
        );
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_rfc_empty_section_returns_warning() {
        let policy = policy_with_creation_rules();
        let body = "## Goal\n\n## Non-goals\ntext\n## Design\ntext";
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            Some(body),
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].severity, Severity::Warning);
        assert_eq!(violations[0].rule, "body_section_empty");
    }

    #[test]
    fn check_create_rfc_section_with_subheadings_not_empty() {
        let policy = policy_with_creation_rules();
        let body =
            "## Goal\ntext\n## Non-goals\ntext\n## Design\n\n### Option A\nDetails\n\n### Option B\nMore";
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            Some(body),
        );
        assert!(
            violations.is_empty(),
            "sub-headings should count as content: {violations:?}"
        );
    }

    #[test]
    fn check_create_issue_no_body_allowed() {
        let policy = policy_with_creation_rules();
        let violations = check_create(&policy, Lifecycle::Execution, &["bug".into()], "Bug", None);
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_no_policy_allows_everything() {
        let policy = Policy::default();
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            None,
        );
        assert!(violations.is_empty());
    }

    #[test]
    fn check_create_case_insensitive_heading_match() {
        let policy = policy_with_creation_rules();
        let body = "# goal\ntext\n### NON-GOALS\ntext\n## design\ntext";
        let violations = check_create(
            &policy,
            Lifecycle::Proposal,
            &["cross-cutting".into()],
            "Test",
            Some(body),
        );
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

    // Regression: state events store 2.0 canonical names ("review",
    // "done", "working"), but legacy policies may carry 1.x names
    // ("under-review", "reviewing", "closed", "implementing"). Both
    // sides of the allow-list comparison must normalize.
    #[test]
    fn check_evidence_matches_across_1x_2x_state_names() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec!["under-review".into(), "reviewing".into(), "closed".into()],
            }),
            ..Default::default()
        };
        // 2.0 thread.status = "review" against 1.x policy entries.
        assert!(check_evidence(&policy, "review").is_empty());
        // 2.0 thread.status = "done" against 1.x "closed".
        assert!(check_evidence(&policy, "done").is_empty());
        // Reverse: 1.x policy lookup against 2.0 status that has no equivalent
        // should still block.
        assert_eq!(check_evidence(&policy, "rejected").len(), 1);
    }

    // Regression for @ltojzq9l: a default policy using only 2.0
    // canonical state names must allow evidence in `review` for an
    // execution-lifecycle thread (issue/task) without needing the
    // legacy "reviewing" entry as well.
    #[test]
    fn check_evidence_canonical_policy_canonical_status_no_friction() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec![
                    "draft".into(),
                    "open".into(),
                    "working".into(),
                    "review".into(),
                    "done".into(),
                    "rejected".into(),
                    "deprecated".into(),
                ],
            }),
            ..Default::default()
        };
        assert!(check_evidence(&policy, "review").is_empty());
        assert!(check_evidence(&policy, "working").is_empty());
        assert!(check_evidence(&policy, "done").is_empty());
    }

    // Regression for @ltojzq9l: error hints normalize the listed
    // policy entries to 2.0 canonical so the user never reads a
    // contradictory "evidence not allowed in 'review'; allowed in
    // ..., reviewing, ..." message.
    #[test]
    fn check_evidence_hint_lists_canonical_state_names() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec!["under-review".into(), "reviewing".into()],
            }),
            ..Default::default()
        };
        // "rejected" is not in the policy and not equivalent to anything
        // listed, so it should produce a violation we can inspect.
        let violations = check_evidence(&policy, "rejected");
        assert_eq!(violations.len(), 1);
        let hint = violations[0].hint.as_ref().unwrap();
        // Normalized, both entries collapse to "review".
        assert!(
            hint.contains("review") && !hint.contains("reviewing"),
            "hint should list canonical 2.0 names; got: {hint}"
        );
        // Violation message uses the canonical form of the offending
        // status so it doesn't conflict with the hint vocabulary.
        assert!(violations[0].reason.contains("'rejected'"));
    }

    #[test]
    fn check_revise_matches_across_1x_2x_state_names() {
        let policy = Policy {
            revise_rules: Some(super::super::policy::ReviseRules {
                allow_body_revise: vec!["implementing".into(), "designing".into()],
                allow_node_revise: vec!["reviewing".into()],
            }),
            ..Default::default()
        };
        // 2.0 "working" matches 1.x "implementing"/"designing".
        assert!(check_revise(&policy, "working", true).is_empty());
        // 2.0 "review" matches 1.x "reviewing".
        assert!(check_revise(&policy, "review", false).is_empty());
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

    // ---- check_op dispatch ----

    #[test]
    fn check_op_create_dispatches_to_creation_rules() {
        let policy = policy_with_creation_rules();
        let v = check_op(
            &policy,
            Op::Create {
                lifecycle: Lifecycle::Proposal,
                tags: &["cross-cutting".into()],
                body: None,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "required_body");
    }

    #[test]
    fn check_op_say_dispatches_to_node_rules() {
        let mut node_rules = HashMap::new();
        node_rules.insert("done".into(), vec![]);
        let policy = Policy {
            node_rules,
            ..Default::default()
        };
        let v = check_op(
            &policy,
            Op::Say {
                status: "done",
                node_type: NodeType::Comment,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "node_type_restricted");
    }

    #[test]
    fn check_op_revise_dispatches_to_revise_rules() {
        let policy = Policy {
            revise_rules: Some(super::super::policy::ReviseRules {
                allow_body_revise: vec!["draft".into()],
                allow_node_revise: vec![],
            }),
            ..Default::default()
        };
        let v = check_op(
            &policy,
            Op::Revise {
                status: "done",
                is_body: true,
            },
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "revise_restricted");
    }

    #[test]
    fn check_op_evidence_dispatches_to_evidence_rules() {
        let policy = Policy {
            evidence_rules: Some(super::super::policy::EvidenceRules {
                allow_evidence: vec!["draft".into()],
            }),
            ..Default::default()
        };
        let v = check_op(&policy, Op::Evidence { status: "done" });
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "evidence_restricted");
    }
}
