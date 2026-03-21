mod support;

use std::collections::HashMap;
use std::fs;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::operation_check::{self, Severity};
use git_forum::internal::policy::{CreationRules, EvidenceRules, Policy, ReviseRules};
use git_forum::internal::state_change;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn empty_policy() -> Policy {
    Policy {
        guards: vec![],
        ..Default::default()
    }
}

fn rfc_creation_policy() -> Policy {
    let mut creation_rules = HashMap::new();
    creation_rules.insert(
        "rfc".into(),
        CreationRules {
            required_body: true,
            body_sections: vec!["Goal".into(), "Non-goals".into(), "Design".into()],
        },
    );
    creation_rules.insert(
        "issue".into(),
        CreationRules {
            required_body: false,
            body_sections: vec![],
        },
    );
    Policy {
        creation_rules,
        ..Default::default()
    }
}

fn restrictive_policy() -> Policy {
    let mut creation_rules = HashMap::new();
    creation_rules.insert(
        "rfc".into(),
        CreationRules {
            required_body: true,
            body_sections: vec!["Goal".into()],
        },
    );

    let mut node_rules = HashMap::new();
    node_rules.insert(
        "draft".into(),
        vec![
            NodeType::Claim,
            NodeType::Question,
            NodeType::Objection,
            NodeType::Evidence,
            NodeType::Summary,
            NodeType::Action,
            NodeType::Risk,
            NodeType::Review,
        ],
    );
    node_rules.insert("accepted".into(), vec![]);
    node_rules.insert("rejected".into(), vec![]);

    Policy {
        creation_rules,
        node_rules,
        revise_rules: Some(ReviseRules {
            allow_body_revise: vec!["draft".into(), "proposed".into(), "open".into()],
            allow_node_revise: vec!["draft".into(), "proposed".into(), "open".into()],
        }),
        evidence_rules: Some(EvidenceRules {
            allow_evidence: vec!["draft".into(), "proposed".into(), "open".into()],
        }),
        ..Default::default()
    }
}

// ---- check_create integration ----

#[test]
fn create_rfc_no_body_blocked_by_policy() {
    let policy = rfc_creation_policy();
    let violations = operation_check::check_create(&policy, ThreadKind::Rfc, "Test", None);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, Severity::Error);
    assert_eq!(violations[0].rule, "required_body");
}

#[test]
fn create_rfc_partial_body_warns() {
    let policy = rfc_creation_policy();
    let violations = operation_check::check_create(
        &policy,
        ThreadKind::Rfc,
        "Test",
        Some("## Goal\nSome goal text"),
    );
    // Missing Non-goals and Design
    assert_eq!(violations.len(), 2);
    assert!(violations.iter().all(|v| v.severity == Severity::Warning));
}

#[test]
fn create_rfc_full_body_passes() {
    let policy = rfc_creation_policy();
    let body = "## Goal\ntext\n## Non-goals\ntext\n## Design\ntext";
    let violations = operation_check::check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
    assert!(violations.is_empty());
}

#[test]
fn create_issue_no_body_allowed() {
    let policy = rfc_creation_policy();
    let violations = operation_check::check_create(&policy, ThreadKind::Issue, "Bug", None);
    assert!(violations.is_empty());
}

#[test]
fn no_policy_allows_everything() {
    let policy = Policy::default();
    assert!(operation_check::check_create(&policy, ThreadKind::Rfc, "Test", None).is_empty());
    assert!(operation_check::check_say(&policy, "accepted", NodeType::Claim).is_empty());
    assert!(operation_check::check_revise(&policy, "accepted", true).is_empty());
    assert!(operation_check::check_evidence(&policy, "accepted").is_empty());
}

// ---- check_say integration ----

#[test]
fn say_on_accepted_rfc_blocked() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("## Goal\nbody"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // Move to accepted
    for new_state in &["proposed", "under-review", "accepted"] {
        state_change::change_state(
            &git,
            &thread_id,
            new_state,
            &["human/alice".into()],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "accepted");

    let policy = restrictive_policy();
    let violations = operation_check::check_say(&policy, &state.status, NodeType::Claim);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, Severity::Error);
    assert_eq!(violations[0].rule, "node_type_restricted");
}

#[test]
fn say_on_draft_rfc_allowed() {
    let (_repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("## Goal\nbody"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "draft");

    let policy = restrictive_policy();
    let violations = operation_check::check_say(&policy, &state.status, NodeType::Claim);
    assert!(violations.is_empty());
}

// ---- check_revise integration ----

#[test]
fn revise_body_on_accepted_blocked() {
    let policy = restrictive_policy();
    let violations = operation_check::check_revise(&policy, "accepted", true);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, Severity::Error);
    assert_eq!(violations[0].rule, "revise_restricted");
}

#[test]
fn revise_body_on_draft_allowed() {
    let policy = restrictive_policy();
    let violations = operation_check::check_revise(&policy, "draft", true);
    assert!(violations.is_empty());
}

#[test]
fn revise_node_on_accepted_blocked() {
    let policy = restrictive_policy();
    let violations = operation_check::check_revise(&policy, "accepted", false);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, Severity::Error);
}

// ---- check_evidence integration ----

#[test]
fn evidence_on_accepted_blocked() {
    let policy = restrictive_policy();
    let violations = operation_check::check_evidence(&policy, "accepted");
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, Severity::Error);
    assert_eq!(violations[0].rule, "evidence_restricted");
}

#[test]
fn evidence_on_draft_allowed() {
    let policy = restrictive_policy();
    let violations = operation_check::check_evidence(&policy, "draft");
    assert!(violations.is_empty());
}

// ---- --force and strict mode ----

#[test]
fn force_does_not_bypass_error() {
    let violations = vec![operation_check::OperationViolation {
        severity: Severity::Error,
        rule: "required_body".into(),
        reason: "rfc threads require a body".into(),
        hint: None,
        fix_command: None,
    }];
    let (has_errors, _output) = operation_check::evaluate_violations(&violations, true, false);
    assert!(has_errors);
}

#[test]
fn strict_mode_promotes_warning_to_error() {
    let violations = vec![operation_check::OperationViolation {
        severity: Severity::Warning,
        rule: "body_section".into(),
        reason: "missing section".into(),
        hint: None,
        fix_command: None,
    }];
    let (has_errors, _output) = operation_check::evaluate_violations(&violations, false, true);
    assert!(has_errors);
}

#[test]
fn strict_mode_force_downgrades_warning_back() {
    let violations = vec![operation_check::OperationViolation {
        severity: Severity::Warning,
        rule: "body_section".into(),
        reason: "missing section".into(),
        hint: None,
        fix_command: None,
    }];
    let (has_errors, _output) = operation_check::evaluate_violations(&violations, true, true);
    assert!(!has_errors);
}

// ---- policy deserialization backward compat ----

#[test]
fn existing_policy_without_new_sections_works() {
    let (_repo, _git, paths) = setup();
    let policy_path = paths.dot_forum.join("policy.toml");
    // Write a policy with only guards (the old format)
    fs::write(
        &policy_path,
        r#"
[[guards]]
on = "under-review->accepted"
requires = ["no_open_objections", "at_least_one_summary"]
"#,
    )
    .unwrap();
    let policy = Policy::load(&policy_path).unwrap();
    assert_eq!(policy.guards.len(), 1);
    assert!(policy.creation_rules.is_empty());
    assert!(policy.node_rules.is_empty());
    assert!(policy.revise_rules.is_none());
    assert!(policy.evidence_rules.is_none());
    assert!(!policy.checks.strict);
}

#[test]
fn full_policy_with_all_sections_deserializes() {
    let (_repo, _git, paths) = setup();
    let policy_path = paths.dot_forum.join("policy.toml");
    fs::write(
        &policy_path,
        r#"
[[guards]]
on = "under-review->accepted"
requires = ["no_open_objections"]

[creation_rules.rfc]
required_body = true
body_sections = ["Goal", "Non-goals"]

[creation_rules.issue]
required_body = false

[node_rules]
"draft" = ["claim", "question"]
"accepted" = []

[revise_rules]
allow_body_revise = ["draft"]
allow_node_revise = ["draft", "proposed"]

[evidence_rules]
allow_evidence = ["draft", "open"]

[checks]
strict = true
"#,
    )
    .unwrap();

    let policy = Policy::load(&policy_path).unwrap();
    assert_eq!(policy.guards.len(), 1);
    assert!(policy.creation_rules.contains_key("rfc"));
    assert!(policy.creation_rules.contains_key("issue"));
    assert!(policy.creation_rules["rfc"].required_body);
    assert!(!policy.creation_rules["issue"].required_body);
    assert_eq!(policy.node_rules["draft"].len(), 2);
    assert!(policy.node_rules["accepted"].is_empty());
    assert_eq!(
        policy.revise_rules.as_ref().unwrap().allow_body_revise,
        vec!["draft"]
    );
    assert_eq!(
        policy.evidence_rules.as_ref().unwrap().allow_evidence,
        vec!["draft", "open"]
    );
    assert!(policy.checks.strict);
}

// ---- heading level agnostic matching ----

#[test]
fn body_section_matches_any_heading_level() {
    let policy = rfc_creation_policy();
    let body = "# Goal\ntext\n### Non-goals\ntext\n###### Design\ntext";
    let violations = operation_check::check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
    assert!(violations.is_empty());
}

#[test]
fn body_section_case_insensitive() {
    let policy = rfc_creation_policy();
    let body = "## GOAL\ntext\n## non-goals\ntext\n## dEsIgN\ntext";
    let violations = operation_check::check_create(&policy, ThreadKind::Rfc, "Test", Some(body));
    assert!(violations.is_empty());
}
