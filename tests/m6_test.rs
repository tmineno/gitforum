mod support;

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{Lifecycle, NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::operation_check::{self, Severity};
use git_forum::internal::policy::{
    CreationRules, GuardEntry, GuardRule, LifecycleCreationRules, Policy, ReviseRules,
};
use git_forum::internal::show;
use git_forum::internal::state_change;
use git_forum::internal::thread;
use git_forum::internal::verify;
use git_forum::internal::write_ops;

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

fn dec_guard_policy() -> Policy {
    let mut p = Policy {
        guards: vec![GuardEntry {
            on: "proposed->accepted".into(),
            requires: vec![GuardRule::NoOpenObjections],
            ..Default::default()
        }],
        ..Default::default()
    };
    p.resolve_guard_scopes();
    p
}

fn task_guard_policy() -> Policy {
    let mut p = Policy {
        guards: vec![GuardEntry {
            on: "reviewing->closed".into(),
            requires: vec![GuardRule::NoOpenActions],
            ..Default::default()
        }],
        ..Default::default()
    };
    p.resolve_guard_scopes();
    p
}

fn make_dec(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Dec,
        "Test DEC",
        Some(
            "## Context\nSome context\n## Decision\nUse Redis\n## Rationale\nFast\n## Impact\nNone",
        ),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn make_task(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Task,
        "Test TASK",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

// ---- DEC lifecycle ----

#[test]
fn dec_create_sets_proposed_state() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
    assert_eq!(state.kind, ThreadKind::Dec);
    // SPEC-2.0 §6.2: kind is on the Create event, not the ID.
    assert!(git_forum::internal::id_alloc::is_bare_token(&id));
}

#[test]
fn dec_proposed_to_accepted() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn dec_proposed_to_rejected() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "rejected");
}

#[test]
fn dec_proposed_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    // Unified §3.1: open (was proposed) cannot directly deprecate; the only
    // edges into `deprecated` are from `done` and `rejected`. Walk via
    // accepted (= done).
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_accepted_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_rejected_to_deprecated() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "deprecated");
}

#[test]
fn dec_proposed_to_pending_is_invalid() {
    // SPEC-2.0 §3.1.1: record lifecycle excludes `working` (and 1.x
    // `pending` normalizes to working). DEC threads cannot enter the
    // working state. (The 1.x test `dec_accepted_to_proposed_is_invalid`
    // is no longer applicable: unified §3.1 includes `done→open` for
    // reopening records.)
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let result = state_change::change_state(
        &git,
        &id,
        "pending",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
}

#[test]
fn dec_proposed_to_accepted_blocked_by_objection() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    write_ops::say_node(
        &git,
        &id,
        NodeType::Objection,
        "Missing benchmarks",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let result = state_change::change_state(
        &git,
        &id,
        "accepted",
        &[],
        "human/alice",
        &fixed_clock(),
        &dec_guard_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_objections"));
}

// ---- TASK lifecycle ----

#[test]
fn task_create_sets_open_state() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
    assert_eq!(state.kind, ThreadKind::Task);
    assert!(git_forum::internal::id_alloc::is_bare_token(&id));
}

#[test]
fn task_full_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    // Unified §3.1: task lifecycle (execution) folds 1.x designing /
    // implementing into a single `working` state, so the full path is
    // open → working → review → done. (1.x `closed` normalizes to `done`.)
    for target in &["working", "review", "closed"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

#[test]
fn task_fast_track_open_to_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "done");
}

// 1.x had separate `designing` / `implementing` states inside Task; both
// fold to `working` in the unified §3.1 graph, so the back-transition
// from implementing to designing is no longer expressible. The remaining
// back-transition coverage runs review → working below.

#[test]
fn task_back_transition_review_to_working() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    // Walk open → working → review, then back-transition review → working
    // (the unified §3.1 review→working edge).
    for target in &["working", "review"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    state_change::change_state(
        &git,
        &id,
        "working",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "working");
}

#[test]
fn task_invalid_open_to_deprecated() {
    // Unified §3.1 has no open→deprecated edge (deprecated is reachable
    // only from done / rejected). open→reviewing was the 1.x invalid edge
    // for tasks; under unified that normalizes to open→review which is
    // valid, so the equivalent invalid case becomes open→deprecated.
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let result = state_change::change_state(
        &git,
        &id,
        "deprecated",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
}

#[test]
fn task_invalid_review_to_draft() {
    // Execution lifecycle excludes `draft`; 1.x reviewing→designing was
    // the negative case (now valid under unified review→working). The
    // equivalent invalid edge for execution is review→draft.
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    for target in &["working", "review"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    let result = state_change::change_state(
        &git,
        &id,
        "draft",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
}

#[test]
fn task_reopen_from_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "open",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn task_reopen_from_rejected() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    state_change::change_state(
        &git,
        &id,
        "rejected",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        &git,
        &id,
        "open",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn task_reviewing_to_closed_blocked_by_actions() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    for target in &["working", "review"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    write_ops::say_node(
        &git,
        &id,
        NodeType::Action,
        "Add tests",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let result = state_change::change_state(
        &git,
        &id,
        "closed",
        &[],
        "human/alice",
        &fixed_clock(),
        &task_guard_policy(),
        state_change::StateChangeOptions::default(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no_open_actions"));
}

// ---- Node types ----

#[test]
fn node_add_alternative() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let node_id = write_ops::say_node(
        &git,
        &id,
        NodeType::Alternative,
        "Use Memcached instead",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_id, node_id);
    // SPEC-2.0 §2.5: `Alternative` is a legacy rhetorical type that
    // canonicalizes to `Comment` on write; the label is preserved.
    assert_eq!(state.nodes[0].node_type, NodeType::Comment);
    assert_eq!(
        state.nodes[0].legacy_subtype.as_deref(),
        Some("alternative")
    );
    assert_eq!(state.nodes[0].body, "Use Memcached instead");
}

#[test]
fn node_add_assumption() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let node_id = write_ops::say_node(
        &git,
        &id,
        NodeType::Assumption,
        "Redis cluster is available in prod",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_id, node_id);
    // SPEC-2.0 §2.5: `Assumption` canonicalizes to `Comment` on write.
    assert_eq!(state.nodes[0].node_type, NodeType::Comment);
    assert_eq!(state.nodes[0].legacy_subtype.as_deref(), Some("assumption"));
    assert_eq!(state.nodes[0].body, "Redis cluster is available in prod");
}

// ---- Verify ----

#[test]
fn verify_dec_proposed_checks_accepted() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    write_ops::say_node(
        &git,
        &id,
        NodeType::Objection,
        "Missing rationale",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let report = verify::verify_thread(&git, &id, &dec_guard_policy()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_objections"));
}

#[test]
fn verify_task_reviewing_checks_closed() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    for target in &["working", "review"] {
        state_change::change_state(
            &git,
            &id,
            target,
            &[],
            "human/alice",
            &fixed_clock(),
            &empty_policy(),
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
    write_ops::say_node(
        &git,
        &id,
        NodeType::Action,
        "Write tests",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();
    let report = verify::verify_thread(&git, &id, &task_guard_policy()).unwrap();
    assert!(!report.passed());
    assert!(report
        .violations
        .iter()
        .any(|v| v.rule == "no_open_actions"));
}

#[test]
fn verify_task_open_passes_trivially() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let report = verify::verify_thread(&git, &id, &task_guard_policy()).unwrap();
    assert!(report.passed());
}

// ---- Ls filter ----

#[test]
fn ls_filters_by_dec_kind() {
    let (_repo, git, _paths) = setup();
    make_dec(&git);
    make_task(&git);
    let ids = thread::list_thread_ids(&git).unwrap();
    let all: Vec<_> = ids
        .iter()
        .map(|id| thread::replay_thread(&git, id).unwrap())
        .collect();
    let decs: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Dec).collect();
    let tasks: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Task).collect();
    assert_eq!(decs.len(), 1);
    assert_eq!(tasks.len(), 1);
}

#[test]
fn ls_filters_by_task_kind() {
    let (_repo, git, _paths) = setup();
    make_task(&git);
    make_task(&git);
    make_dec(&git);
    let ids = thread::list_thread_ids(&git).unwrap();
    let all: Vec<_> = ids
        .iter()
        .map(|id| thread::replay_thread(&git, id).unwrap())
        .collect();
    let tasks: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Task).collect();
    assert_eq!(tasks.len(), 2);
}

// ---- Operation check: creation rules ----

#[test]
fn dec_create_requires_body() {
    let dec_policy = Policy {
        creation_rules: {
            let mut m = HashMap::new();
            m.insert(
                "record".into(),
                LifecycleCreationRules {
                    base: CreationRules {
                        required_body: true,
                        body_sections: vec!["Context".into(), "Decision".into()],
                    },
                    tag: HashMap::new(),
                },
            );
            m
        },
        ..Default::default()
    };
    let violations =
        operation_check::check_create(&dec_policy, Lifecycle::Record, &[], "Test", None);
    assert!(violations.iter().any(|v| v.severity == Severity::Error));
}

#[test]
fn dec_create_missing_sections_warns() {
    let dec_policy = Policy {
        creation_rules: {
            let mut m = HashMap::new();
            m.insert(
                "record".into(),
                LifecycleCreationRules {
                    base: CreationRules {
                        required_body: true,
                        body_sections: vec![
                            "Context".into(),
                            "Decision".into(),
                            "Rationale".into(),
                            "Impact".into(),
                        ],
                    },
                    tag: HashMap::new(),
                },
            );
            m
        },
        ..Default::default()
    };
    // Body provided but missing sections
    let violations = operation_check::check_create(
        &dec_policy,
        Lifecycle::Record,
        &[],
        "Test",
        Some("## Context\nSome context\n## Decision\nUse Redis"),
    );
    // Should have warnings for missing Rationale and Impact, but no errors
    let errors: Vec<_> = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .collect();
    let warnings: Vec<_> = violations
        .iter()
        .filter(|v| v.severity == Severity::Warning)
        .collect();
    assert!(errors.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn task_create_no_body_succeeds() {
    let task_policy = Policy {
        creation_rules: {
            let mut m = HashMap::new();
            m.insert(
                "execution".into(),
                LifecycleCreationRules {
                    base: CreationRules::default(),
                    tag: {
                        let mut t = HashMap::new();
                        t.insert(
                            "task".into(),
                            CreationRules {
                                required_body: false,
                                body_sections: vec![
                                    "Background".into(),
                                    "Acceptance criteria".into(),
                                ],
                            },
                        );
                        t
                    },
                },
            );
            m
        },
        ..Default::default()
    };
    let violations = operation_check::check_create(
        &task_policy,
        Lifecycle::Execution,
        &["task".into()],
        "Test",
        None,
    );
    assert!(violations.is_empty());
}

#[test]
fn task_create_with_body_missing_sections_warns() {
    let task_policy = Policy {
        creation_rules: {
            let mut m = HashMap::new();
            m.insert(
                "execution".into(),
                LifecycleCreationRules {
                    base: CreationRules::default(),
                    tag: {
                        let mut t = HashMap::new();
                        t.insert(
                            "task".into(),
                            CreationRules {
                                required_body: false,
                                body_sections: vec![
                                    "Background".into(),
                                    "Acceptance criteria".into(),
                                    "Exceptions".into(),
                                ],
                            },
                        );
                        t
                    },
                },
            );
            m
        },
        ..Default::default()
    };
    let violations = operation_check::check_create(
        &task_policy,
        Lifecycle::Execution,
        &["task".into()],
        "Test",
        Some("## Background\nSome background"),
    );
    let errors: Vec<_> = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .collect();
    let warnings: Vec<_> = violations
        .iter()
        .filter(|v| v.severity == Severity::Warning)
        .collect();
    assert!(errors.is_empty());
    assert!(!warnings.is_empty());
}

// ---- Revise rules ----

#[test]
fn body_revise_dec_accepted_blocked() {
    let policy = Policy {
        revise_rules: Some(ReviseRules {
            allow_body_revise: vec![
                "draft".into(),
                "proposed".into(),
                "open".into(),
                "pending".into(),
                "designing".into(),
                "implementing".into(),
            ],
            allow_node_revise: vec![],
        }),
        ..Default::default()
    };
    let violations = operation_check::check_revise(&policy, "accepted", true);
    assert!(violations.iter().any(|v| v.severity == Severity::Error));
}

#[test]
fn body_revise_task_designing_allowed() {
    let policy = Policy {
        revise_rules: Some(ReviseRules {
            allow_body_revise: vec![
                "draft".into(),
                "proposed".into(),
                "open".into(),
                "pending".into(),
                "designing".into(),
                "implementing".into(),
            ],
            allow_node_revise: vec![],
        }),
        ..Default::default()
    };
    let violations = operation_check::check_revise(&policy, "designing", true);
    assert!(violations.is_empty());
}

// ---- Show renders DEC and TASK ----

#[test]
fn show_dec_includes_kind() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, false);
    assert!(output.contains("**kind:**     dec"));
    assert!(output.contains("**status:**   open"));
}

#[test]
fn show_task_includes_kind() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, false);
    assert!(output.contains("**kind:**     task"));
    assert!(output.contains("**status:**   open"));
}
