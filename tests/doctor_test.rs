//! Module integration tests for `src/internal/doctor.rs`
//! (test-policy.md category 1). Track G's "parent done but children
//! open" advisory tests will land here in a later split.

mod support;

use chrono::Utc;
use git_forum::internal::commands::doctor::{self, CheckLevel};
use git_forum::internal::event::ThreadKind;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;

use support::forum::{drive_to_done, link_thread, make_thread, setup_no_init as setup};

#[test]
fn doctor_after_init_all_pass() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.all_passed(),
        "doctor should pass after init: {:?}",
        report
            .checks
            .iter()
            .filter(|c| !c.passed())
            .map(|c| (&c.name, &c.detail))
            .collect::<Vec<_>>()
    );
}

#[test]
fn doctor_uninitialized_reports_failures() {
    let (_repo, git, paths) = setup();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(!report.all_passed());
}

#[test]
fn doctor_reports_missing_template_files() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    std::fs::remove_file(paths.dot_forum.join("templates/dec.md")).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let dec_check = report
        .checks
        .iter()
        .find(|c| c.name == "template dec.md")
        .expect("should have dec.md check");
    assert_eq!(dec_check.level, CheckLevel::Fail);
    let issue_check = report
        .checks
        .iter()
        .find(|c| c.name == "template issue.md")
        .expect("should have issue.md check");
    assert_eq!(issue_check.level, CheckLevel::Ok);
}

#[test]
fn doctor_reports_empty_template_file() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    std::fs::write(paths.dot_forum.join("templates/rfc.md"), "").unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let rfc_check = report
        .checks
        .iter()
        .find(|c| c.name == "template rfc.md")
        .expect("should have rfc.md check");
    assert_eq!(rfc_check.level, CheckLevel::Fail);
}

// SQLite index health checks (`index database` / `index integrity` /
// `index freshness`) were removed at Phase 2 slot 8 (RFC `7ymtc4b2`)
// when the doctor rewired to SPEC-3.0 §11/§12 snapshot integrity per
// ADR-011 Decision 6 (no index in v3.0.0). The matching
// `doctor_warns_on_missing_index` / `doctor_passes_index_after_reindex` /
// `doctor_warns_on_stale_index` tests were dropped in the same commit.

// ---- Linked-thread advisory (Track G) ----

#[test]
fn doctor_surfaces_open_implementing_children_under_done_parent() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let parent = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    let child_open = make_thread(&git, ThreadKind::Task, "Still working");
    let child_done = make_thread(&git, ThreadKind::Task, "Already done");
    link_thread(&git, &child_open, &parent, "implements");
    link_thread(&git, &child_done, &parent, "implements");

    drive_to_done(&git, &policy, &parent);
    drive_to_done(&git, &policy, &child_done);

    let report = doctor::run_doctor(&git, &paths).unwrap();

    let advisory_match = report
        .advisories
        .iter()
        .find(|a| a.contains(&parent))
        .unwrap_or_else(|| {
            panic!(
                "expected advisory for {parent}, got: {:?}",
                report.advisories
            )
        });
    assert!(
        advisory_match.contains(&child_open),
        "expected open child id in advisory: {advisory_match}"
    );
    assert!(
        !advisory_match.contains(&child_done),
        "done child should not appear in advisory: {advisory_match}"
    );
    assert!(
        advisory_match.contains("1 implementing child still open"),
        "advisory phrasing changed: {advisory_match}"
    );

    // Advisory MUST NOT affect the doctor's pass/fail decision.
    assert!(
        report.all_passed(),
        "advisories should not flip doctor pass/fail"
    );
}

#[test]
fn doctor_quiet_when_no_done_parents_have_open_implementers() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "Lonely RFC");
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.advisories.is_empty(),
        "no advisories expected: {:?}",
        report.advisories
    );
}

// ---- Orphan ref / prune-orphans (Phase 1: Finding 4) ----

/// Create a commit tree containing a single file with arbitrary contents
/// (not `event.json`), then point a `refs/forum/threads/<id>` ref at it.
/// This emulates the failure mode that surfaced in the v2.0.0 review:
/// `git forum doctor` reporting `[FAIL] replay …` for a ref whose tip
/// commit has no parseable thread history.
fn write_orphan_thread_ref(git: &git_forum::internal::git_ops::GitOps, thread_id: &str) {
    let blob = git.hash_object(b"not an event\n").unwrap();
    let tree = git.mktree_single("not-event.txt", &blob).unwrap();
    let commit = git.commit_tree(&tree, &[], "dummy event").unwrap();
    let ref_name = format!("refs/forum/threads/{thread_id}");
    git.create_ref(&ref_name, &commit).unwrap();
}

#[test]
fn doctor_warns_on_orphan_thread_ref() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    write_orphan_thread_ref(&git, "ASK-orphan1");

    let report = doctor::run_doctor(&git, &paths).unwrap();
    let orphan = report
        .checks
        .iter()
        .find(|c| c.name.contains("orphan ref") && c.name.contains("ASK-orphan1"))
        .unwrap_or_else(|| {
            panic!(
                "expected orphan ref check, got: {:?}",
                report.checks.iter().map(|c| &c.name).collect::<Vec<_>>()
            )
        });
    assert_eq!(orphan.level, CheckLevel::Warn);
    assert!(
        orphan
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("prune-orphans"),
        "expected prune-orphans hint, got: {:?}",
        orphan.detail
    );
    // WARN must not flip pass/fail — only FAIL does.
    assert!(report.all_passed());
}

/// Strict mode promotes silent replay no-ops to FAIL; lenient (default) does
/// not. We forge a thread whose chain ends with a `resolve` event targeting
/// a node that was never created, then assert the WARN/OK boundary differs
/// between the two doctor modes.
#[test]
fn doctor_strict_flips_unknown_target_resolve_to_fail() {
    use chrono::TimeZone;
    use git_forum::internal::clock::FixedClock;
    use git_forum::internal::event::{self as ev, Event, EventType, ThreadKind};

    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let clock = FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    };
    let thread_id = git_forum::internal::create::create_thread(
        &git,
        ThreadKind::Issue,
        "Phantom resolve",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();

    // Append a resolve event whose target_node_id never had a Say event.
    let resolve = Event {
        thread_id: thread_id.clone(),
        event_type: EventType::Resolve,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/alice".into(),
        target_node_id: Some("ghost-node-id".into()),
        ..Event::default()
    };
    ev::write_event(&git, &resolve).unwrap();

    // Lenient (default): the orphan resolve is silent → no FAIL.
    let lenient = doctor::run_doctor(&git, &paths).unwrap();
    let lenient_strict_fails: Vec<_> = lenient
        .checks
        .iter()
        .filter(|c| c.name.starts_with("strict-replay"))
        .collect();
    assert!(
        lenient_strict_fails.is_empty(),
        "lenient mode should not emit strict-replay checks: {:?}",
        lenient_strict_fails
            .iter()
            .map(|c| (&c.name, &c.detail))
            .collect::<Vec<_>>()
    );

    // Strict: resolve-of-unknown-node must surface as FAIL.
    let strict = doctor::run_doctor_strict(&git, &paths).unwrap();
    let strict_fail = strict
        .checks
        .iter()
        .find(|c| c.name.starts_with("strict-replay") && c.name.contains(&thread_id))
        .unwrap_or_else(|| {
            panic!(
                "strict mode should flag the orphan resolve, got: {:?}",
                strict.checks.iter().map(|c| &c.name).collect::<Vec<_>>()
            )
        });
    assert_eq!(strict_fail.level, CheckLevel::Fail);
    assert!(strict_fail
        .detail
        .as_deref()
        .unwrap_or("")
        .contains("ghost-node-id"));
}

/// `prune-stale-events` round-trip: build a thread with a resolve event whose
/// `target_node_id` doesn't exist, run the planner, apply, then assert the
/// thread no longer triggers strict-replay FAILs and that unrelated history
/// (the create event SHA) is preserved.
#[test]
fn prune_stale_events_drops_unknown_target_resolve() {
    use chrono::TimeZone;
    use git_forum::internal::clock::FixedClock;
    use git_forum::internal::event::{self as ev, Event, EventType, ThreadKind};
    use git_forum::internal::prune;

    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let clock = FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    };
    let thread_id = git_forum::internal::create::create_thread(
        &git,
        ThreadKind::Issue,
        "Phantom resolve",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let original_create_sha = git
        .resolve_ref(&format!("refs/forum/threads/{thread_id}"))
        .unwrap()
        .unwrap();

    // Append a resolve event whose target was never created.
    ev::write_event(
        &git,
        &Event {
            thread_id: thread_id.clone(),
            event_type: EventType::Resolve,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some("ghost-node-id".into()),
            ..Event::default()
        },
    )
    .unwrap();

    // Planner spots exactly one stale event on this thread.
    let plans = prune::scan_stale_events(&git).unwrap();
    let plan = plans
        .iter()
        .find(|p| p.thread_id == thread_id)
        .unwrap_or_else(|| {
            panic!(
                "expected plan for {thread_id}, got: {:?}",
                plans.iter().map(|p| &p.thread_id).collect::<Vec<_>>()
            )
        });
    assert_eq!(plan.events_to_drop.len(), 1);
    assert_eq!(plan.orphan_target_count, 1);

    let dropped = prune::apply_stale_event_plans(&git, &plans).unwrap();
    assert_eq!(dropped, 1);

    // After apply: strict replay reports no issues for this thread.
    let (state, issues) = git_forum::internal::thread::replay_thread_strict(&git, &thread_id)
        .expect("strict replay should succeed");
    assert!(
        issues.is_empty(),
        "strict replay should be clean after prune: {issues:?}"
    );
    assert_eq!(state.id, thread_id);

    // Chain has only the create event again — drop happened on the only
    // post-create commit, so the chain is exactly its original prefix.
    let new_tip = git
        .resolve_ref(&format!("refs/forum/threads/{thread_id}"))
        .unwrap()
        .unwrap();
    assert_eq!(
        new_tip, original_create_sha,
        "prune should have rolled the ref back to the create commit"
    );
}

#[test]
fn prune_stale_events_preserves_unaffected_prefix_sha() {
    // Chain: create → resolve(unknown) → comment(real). The stale resolve
    // is dropped; the create commit SHA must be unchanged but the comment
    // commit MUST be re-emitted (its parent changed).
    use chrono::TimeZone;
    use git_forum::internal::clock::FixedClock;
    use git_forum::internal::event::{self as ev, Event, EventType, NodeType, ThreadKind};
    use git_forum::internal::prune;

    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let clock = FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    };
    let thread_id = git_forum::internal::create::create_thread(
        &git,
        ThreadKind::Issue,
        "Mixed",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let create_sha = git
        .resolve_ref(&format!("refs/forum/threads/{thread_id}"))
        .unwrap()
        .unwrap();
    ev::write_event(
        &git,
        &Event {
            thread_id: thread_id.clone(),
            event_type: EventType::Resolve,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some("ghost".into()),
            ..Event::default()
        },
    )
    .unwrap();
    ev::write_event(
        &git,
        &Event {
            thread_id: thread_id.clone(),
            event_type: EventType::Say,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 2, 0).unwrap(),
            actor: "human/alice".into(),
            node_type: Some(NodeType::Comment),
            body: Some("real comment".into()),
            ..Event::default()
        },
    )
    .unwrap();

    let plans = prune::scan_stale_events(&git).unwrap();
    prune::apply_stale_event_plans(&git, &plans).unwrap();

    // After prune: chain length is 2 (create + comment); create SHA stable.
    let new_chain = git
        .rev_list(&format!("refs/forum/threads/{thread_id}"))
        .unwrap();
    assert_eq!(new_chain.len(), 2, "expected 2 commits, got: {new_chain:?}");
    assert_eq!(
        new_chain.last().unwrap(),
        &create_sha,
        "create commit must keep its original SHA"
    );

    // Comment is preserved (not dropped) and visible.
    let (state, issues) =
        git_forum::internal::thread::replay_thread_strict(&git, &thread_id).unwrap();
    assert!(issues.is_empty());
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].body, "real comment");
}

#[test]
fn prune_orphans_scan_finds_and_delete_removes() {
    use git_forum::internal::prune;
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    // One real thread (must survive) and one orphan ref (must be deleted).
    let real_id = make_thread(&git, ThreadKind::Rfc, "Real RFC");
    write_orphan_thread_ref(&git, "ASK-orphan2");

    let orphans = prune::scan(&git).unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "got: {:?}",
        orphans.iter().map(|o| &o.thread_id).collect::<Vec<_>>()
    );
    assert_eq!(orphans[0].thread_id, "ASK-orphan2");

    prune::delete(&git, &orphans).unwrap();

    // Real thread still resolvable; orphan ref gone.
    let post = prune::scan(&git).unwrap();
    assert!(
        post.is_empty(),
        "orphans remained after delete: {:?}",
        post.iter().map(|o| &o.thread_id).collect::<Vec<_>>()
    );
    let ids = git_forum::internal::thread::list_thread_ids(&git).unwrap();
    assert!(ids.contains(&real_id), "real thread missing after prune");
    assert!(!ids.iter().any(|i| i == "ASK-orphan2"));
}
