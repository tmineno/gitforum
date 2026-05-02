//! Track C — `git forum migrate` integration tests (JOB-e216r3on).
//!
//! Cover the acceptance criteria spelled out in the task body:
//! - ref rewrite + alias preservation
//! - synthetic `facet_set` per migrated thread
//! - lossless state round-trip across all four 1.x kinds
//! - node-event canonicalization with `legacy_subtype` preserved
//! - `--dry-run` makes no changes
//! - re-running is a no-op (idempotent)
//! - policy.toml auto-rewrite + at_least_one_summary warning
//! - shipped-script subcommand-grouping warning

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::{self, Event, EventType, NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::migrate;
use git_forum::internal::refs;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn create_event(thread_id: &str, kind: ThreadKind, title: &str, body: &str) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        title: Some(title.into()),
        kind: Some(kind),
        body: Some(body.into()),
        ..Event::default()
    }
}

fn say_event(thread_id: &str, node_type: NodeType, body: &str, ts_offset_min: i64) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::Say,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::minutes(ts_offset_min),
        actor: "human/bob".into(),
        node_type: Some(node_type),
        body: Some(body.into()),
        ..Event::default()
    }
}

fn state_event(thread_id: &str, new_state: &str, ts_offset_min: i64) -> Event {
    Event {
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::minutes(ts_offset_min),
        actor: "human/alice".into(),
        new_state: Some(new_state.into()),
        ..Event::default()
    }
}

/// Build a 1.x thread chain (Create + extra events) at `legacy_id`.
fn build_legacy_thread(git: &GitOps, legacy_id: &str, kind: ThreadKind, evs: Vec<Event>) {
    let create = create_event(legacy_id, kind, "Test thread", "body");
    event::write_event(git, &create).unwrap();
    for ev in evs {
        event::write_event(git, &ev).unwrap();
    }
}

#[test]
fn migrate_rewrites_opaque_ref_to_bare_token() {
    let (_repo, git, paths) = setup();
    let legacy = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc,
        "human/alice",
        "Test",
        "2026-01-01T00:00:00Z",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, vec![]);
    assert!(legacy.starts_with("RFC-"));

    let outcome = migrate::run(&git, &paths, "system/migrate", false).unwrap();
    assert_eq!(outcome.threads_migrated, 1);
    assert_eq!(outcome.threads_skipped, 0);

    // Canonical ref points to the bare token.
    let token = legacy.strip_prefix("RFC-").unwrap();
    let canonical = git
        .resolve_ref(&refs::thread_ref(token))
        .unwrap()
        .expect("canonical ref should exist after migrate");
    let alias = git
        .resolve_ref(&migrate::alias_ref(&legacy))
        .unwrap()
        .expect("alias ref should exist after migrate");
    assert_eq!(
        canonical, alias,
        "alias and canonical should target same tip"
    );
    // Original kind-prefixed ref is no longer in refs/forum/threads/.
    assert!(git
        .resolve_ref(&refs::thread_ref(&legacy))
        .unwrap()
        .is_none());
    // list_thread_ids returns the bare-token form only.
    let ids = thread::list_thread_ids(&git).unwrap();
    assert_eq!(ids, vec![token.to_string()]);
}

#[test]
fn migrate_preserves_legacy_id_resolution_via_alias() {
    let (_repo, git, paths) = setup();
    let legacy = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Issue,
        "human/alice",
        "Test",
        "2026-01-01T00:00:00Z",
        &[2, 3, 4, 5, 6, 7, 8, 9],
    );
    build_legacy_thread(&git, &legacy, ThreadKind::Issue, vec![]);
    let token = legacy.strip_prefix("ASK-").unwrap().to_string();

    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    // Resolving the legacy ID hits the alias and returns the canonical token.
    let resolved = thread::resolve_thread_id(&git, &legacy).unwrap();
    assert_eq!(resolved, token);
    // Lowercased legacy form also works.
    let resolved_ci = thread::resolve_thread_id(&git, &legacy.to_lowercase()).unwrap();
    assert_eq!(resolved_ci, token);
}

#[test]
fn migrate_appends_facet_set_with_conventional_tags() {
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0001".to_string();
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, vec![]);

    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    let token = thread::resolve_thread_id(&git, &legacy).unwrap();
    let evs = event::load_thread_events(&git, &token).unwrap();
    let facet = evs
        .iter()
        .find(|e| e.event_type == EventType::FacetSet)
        .expect("migrated chain must end with a facet_set event");
    assert_eq!(facet.lifecycle.as_deref(), Some("proposal"));
    assert_eq!(facet.tags_add, vec!["cross-cutting".to_string()]);
    assert_eq!(facet.actor, "system/migrate");
}

#[test]
fn migrate_appends_facet_set_for_each_kind() {
    let (_repo, git, paths) = setup();
    // One thread per kind.
    let kinds = [
        (
            ThreadKind::Rfc,
            "RFC-0001",
            "proposal",
            vec!["cross-cutting"],
        ),
        (ThreadKind::Issue, "ASK-0001", "execution", vec!["bug"]),
        (ThreadKind::Task, "JOB-0001", "execution", vec!["task"]),
        (ThreadKind::Dec, "DEC-0001", "record", vec![]),
    ];
    for (kind, id, _, _) in &kinds {
        build_legacy_thread(&git, id, *kind, vec![]);
    }

    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    for (_, legacy, lifecycle, tags) in &kinds {
        let canonical = thread::resolve_thread_id(&git, legacy).unwrap();
        let evs = event::load_thread_events(&git, &canonical).unwrap();
        let facet = evs
            .iter()
            .find(|e| e.event_type == EventType::FacetSet)
            .expect("expected facet_set after migration");
        assert_eq!(
            facet.lifecycle.as_deref(),
            Some(*lifecycle),
            "kind {legacy}"
        );
        assert_eq!(
            facet.tags_add,
            tags.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "kind {legacy}"
        );
    }
}

#[test]
fn migrate_canonicalizes_legacy_node_types_with_subtype() {
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0007".to_string();
    let evs = vec![
        say_event(&legacy, NodeType::Question, "what about X?", 1),
        say_event(&legacy, NodeType::Summary, "TLDR", 2),
        say_event(&legacy, NodeType::Action, "ship it", 3),
    ];
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, evs);

    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    let token = thread::resolve_thread_id(&git, &legacy).unwrap();
    let migrated = event::load_thread_events(&git, &token).unwrap();
    let say_events: Vec<&Event> = migrated
        .iter()
        .filter(|e| e.event_type == EventType::Say)
        .collect();
    assert_eq!(say_events.len(), 3);
    // Question → Comment + legacy_subtype="question"
    assert_eq!(say_events[0].node_type, Some(NodeType::Comment));
    assert_eq!(say_events[0].legacy_subtype.as_deref(), Some("question"));
    // Summary → Comment + legacy_subtype="summary"
    assert_eq!(say_events[1].node_type, Some(NodeType::Comment));
    assert_eq!(say_events[1].legacy_subtype.as_deref(), Some("summary"));
    // Action stays Action; no subtype label.
    assert_eq!(say_events[2].node_type, Some(NodeType::Action));
    assert!(say_events[2].legacy_subtype.is_none());
}

#[test]
fn migrate_lossless_state_round_trip_all_kinds() {
    // Acceptance: every state in every 1.x kind round-trips to a defined 2.0 state.
    let cases: &[(ThreadKind, &[&str])] = &[
        (
            ThreadKind::Rfc,
            &[
                "draft",
                "proposed",
                "under-review",
                "accepted",
                "rejected",
                "withdrawn",
                "deprecated",
            ],
        ),
        (
            ThreadKind::Issue,
            &["open", "pending", "closed", "rejected", "withdrawn"],
        ),
        (
            ThreadKind::Dec,
            &[
                "proposed",
                "accepted",
                "rejected",
                "deprecated",
                "withdrawn",
            ],
        ),
        (
            ThreadKind::Task,
            &[
                "open",
                "designing",
                "implementing",
                "reviewing",
                "closed",
                "rejected",
                "withdrawn",
            ],
        ),
    ];
    for (kind, states) in cases {
        for state in *states {
            let migrated = event::migrate_legacy_state(*kind, state);
            assert!(
                kind.lifecycle().allows_state(migrated),
                "kind={kind} state={state} migrated={migrated} not in lifecycle's allowed set"
            );
        }
    }
}

#[test]
fn migrate_state_event_normalization_persists() {
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0042".to_string();
    let evs = vec![state_event(&legacy, "under-review", 1)];
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, evs);

    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    let token = thread::resolve_thread_id(&git, &legacy).unwrap();
    let migrated = event::load_thread_events(&git, &token).unwrap();
    let state_ev = migrated
        .iter()
        .find(|e| e.event_type == EventType::State)
        .unwrap();
    // SPEC-2.0 §3.1.2 normalizes `under-review` to `review`.
    assert_eq!(state_ev.new_state.as_deref(), Some("review"));
}

#[test]
fn migrate_dry_run_does_not_modify_refs() {
    let (_repo, git, paths) = setup();
    let legacy = "ASK-0001".to_string();
    build_legacy_thread(&git, &legacy, ThreadKind::Issue, vec![]);

    let before_canonical = git.resolve_ref(&refs::thread_ref(&legacy)).unwrap();
    let before_alias = git.resolve_ref(&migrate::alias_ref(&legacy)).unwrap();

    let outcome = migrate::run(&git, &paths, "system/migrate", true).unwrap();
    assert_eq!(outcome.threads_migrated, 1);

    let after_canonical = git.resolve_ref(&refs::thread_ref(&legacy)).unwrap();
    let after_alias = git.resolve_ref(&migrate::alias_ref(&legacy)).unwrap();
    assert_eq!(
        before_canonical, after_canonical,
        "legacy ref must not change"
    );
    assert!(
        before_alias.is_none() && after_alias.is_none(),
        "no alias created in dry-run"
    );
}

#[test]
fn migrate_is_idempotent_on_second_run() {
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0001".to_string();
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, vec![]);

    migrate::run(&git, &paths, "system/migrate", false).unwrap();
    let first_token = thread::resolve_thread_id(&git, &legacy).unwrap();
    let first_tip = git
        .resolve_ref(&refs::thread_ref(&first_token))
        .unwrap()
        .expect("post-migrate tip should exist");
    let first_event_count = event::load_thread_events(&git, &first_token).unwrap().len();

    let outcome2 = migrate::run(&git, &paths, "system/migrate", false).unwrap();
    let second_token = thread::resolve_thread_id(&git, &legacy).unwrap();
    let second_tip = git
        .resolve_ref(&refs::thread_ref(&second_token))
        .unwrap()
        .unwrap();
    let second_event_count = event::load_thread_events(&git, &second_token)
        .unwrap()
        .len();

    assert_eq!(outcome2.threads_skipped, 1, "second run is a no-op");
    assert_eq!(outcome2.threads_migrated, 0);
    assert_eq!(first_token, second_token);
    assert_eq!(first_tip, second_tip, "ref tip must not change");
    assert_eq!(
        first_event_count, second_event_count,
        "event chain length must not grow on re-run"
    );
}

#[test]
fn migrate_policy_warns_on_at_least_one_summary() {
    let (_repo, git, paths) = setup();
    // The 2.0 default policy no longer contains the predicate (per ADR-006
    // and @ltojzq9l), so write a legacy 1.x policy explicitly to verify
    // that build_plan still surfaces the deprecation warning when one
    // shows up in user-authored policies.
    let legacy_policy = r#"
[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary"]
"#;
    std::fs::write(paths.dot_forum.join("policy.toml"), legacy_policy).unwrap();
    let plan = migrate::build_plan(&git, &paths).unwrap();
    assert!(
        plan.policy_warnings
            .iter()
            .any(|w| w.contains("at_least_one_summary") && w.contains(".forum/policy.toml")),
        "warnings: {:?}",
        plan.policy_warnings
    );
}

#[test]
fn migrate_rewrites_legacy_creation_rules_in_policy() {
    let (_repo, git, paths) = setup();
    // Default policy.toml has [creation_rules.rfc/issue/dec/task] and a
    // guard with `requires = [..., at_least_one_summary, ...]`.
    let outcome = migrate::run(&git, &paths, "system/migrate", false).unwrap();
    let updated = std::fs::read_to_string(paths.dot_forum.join("policy.toml")).unwrap();
    assert!(updated.contains("[creation_rules.proposal.tag.cross-cutting]"));
    assert!(updated.contains("[creation_rules.execution.tag.bug]"));
    assert!(updated.contains("[creation_rules.execution.tag.task]"));
    assert!(updated.contains("[creation_rules.record]"));
    assert!(!updated.contains("[creation_rules.rfc]"));
    assert!(!updated.contains("[creation_rules.issue]"));
    // The rewrite log records the source line of each translated key.
    assert!(
        outcome
            .policy_rewrites
            .iter()
            .any(|r| r.contains("creation_rules.rfc")),
        "rewrites: {:?}",
        outcome.policy_rewrites
    );
}

#[test]
fn migrate_warns_on_kind_prefixed_subcommand_in_scripts() {
    let (_repo, git, paths) = setup();
    // Plant a helper script that uses the legacy `git forum rfc new` form.
    let helper_dir = paths.dot_forum.join("scripts");
    std::fs::create_dir_all(&helper_dir).unwrap();
    let helper = helper_dir.join("bootstrap.sh");
    std::fs::write(
        &helper,
        "#!/bin/sh\n# Old-style helper.\ngit forum rfc new \"My RFC\"\ngit forum issue close ASK-1\n",
    )
    .unwrap();

    let plan = migrate::build_plan(&git, &paths).unwrap();
    assert_eq!(
        plan.script_warnings.len(),
        2,
        "warnings: {:?}",
        plan.script_warnings
    );
    assert!(plan
        .script_warnings
        .iter()
        .all(|w| w.contains("bootstrap.sh")));
    assert!(plan
        .script_warnings
        .iter()
        .any(|w| w.contains("git forum rfc new")));
    assert!(plan
        .script_warnings
        .iter()
        .any(|w| w.contains("git forum issue close")));
}

#[test]
fn migrate_does_not_double_migrate_already_bare_threads() {
    let (_repo, git, paths) = setup();
    // Already 2.0: bare token, with a facet_set in chain.
    let token = "a7f3b2x1".to_string();
    let mut create = create_event(&token, ThreadKind::Rfc, "Native 2.0", "body");
    create.kind = Some(ThreadKind::Rfc);
    event::write_event(&git, &create).unwrap();
    let facet = Event {
        thread_id: token.clone(),
        event_type: EventType::FacetSet,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 1).unwrap(),
        actor: "human/alice".into(),
        ..Event::default()
    }
    .with_lifecycle("proposal")
    .with_tags_add(vec!["cross-cutting".into()]);
    event::write_event(&git, &facet).unwrap();

    let outcome = migrate::run(&git, &paths, "system/migrate", false).unwrap();
    assert_eq!(outcome.threads_skipped, 1);
    assert_eq!(outcome.threads_migrated, 0);
}

#[test]
fn commit_msg_hook_recognizes_legacy_alias_and_at_marker() {
    // Regression: post-migration, the commit-msg hook's thread-reference
    // checker must accept both the legacy kind-prefixed alias (`JOB-...`)
    // and the 2.0 `@<token>` display form. Before the fix, both were
    // reported as `not found` because the checker only consulted
    // `refs/forum/threads/<id>` and didn't know about the alias namespace.
    use git_forum::internal::hook;
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0001".to_string();
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, vec![]);
    migrate::run(&git, &paths, "system/migrate", false).unwrap();
    let token = thread::resolve_thread_id(&git, &legacy).unwrap();

    // Extracting from a commit message picks up both forms (deduped per form).
    let extracted = hook::extract_thread_ids(&format!("Closes {legacy} (aka @{token})"));
    assert_eq!(extracted, vec![legacy.clone(), token.clone()]);

    let result = hook::check_thread_refs(&git, &extracted).unwrap();
    assert!(
        result.missing_ids.is_empty(),
        "expected both forms to resolve; missing={:?}",
        result.missing_ids
    );
    assert_eq!(result.found_ids.len(), 2);
}

#[test]
fn alias_resolves_after_canonical_thread_advances() {
    // Regression: aliases are frozen at the migration-time tip, but the
    // canonical thread ref keeps moving forward as new events land.
    // Resolution must NOT depend on alias-tip ↔ canonical-tip equality.
    let (_repo, git, paths) = setup();
    let legacy = "RFC-0001".to_string();
    build_legacy_thread(&git, &legacy, ThreadKind::Rfc, vec![]);
    migrate::run(&git, &paths, "system/migrate", false).unwrap();

    let token = thread::resolve_thread_id(&git, &legacy).unwrap();
    // Append a fresh event, advancing the canonical tip past the alias's
    // frozen position.
    let new_event = state_event(&token, "review", 100);
    event::write_event(&git, &new_event).unwrap();

    // Legacy ID still resolves.
    let resolved_after = thread::resolve_thread_id(&git, &legacy).unwrap();
    assert_eq!(resolved_after, token);

    // Sanity: the alias is now stale (different SHA from canonical).
    let canonical_tip = git.resolve_ref(&refs::thread_ref(&token)).unwrap().unwrap();
    let alias_tip = git
        .resolve_ref(&migrate::alias_ref(&legacy))
        .unwrap()
        .unwrap();
    assert_ne!(
        canonical_tip, alias_tip,
        "test premise: alias must be stale relative to canonical after a new event"
    );
}

#[test]
fn migrate_populates_round_trip_token_map() {
    let ids = vec![
        "RFC-0001".into(),
        "RFC-a7f3b2x1".into(),
        "ASK-q3kfj49v".into(),
    ];
    let map = migrate::predicted_token_map(&ids);
    assert_eq!(
        map.get("RFC-a7f3b2x1").map(String::as_str),
        Some("a7f3b2x1")
    );
    assert_eq!(
        map.get("ASK-q3kfj49v").map(String::as_str),
        Some("q3kfj49v")
    );
    let derived = map.get("RFC-0001").unwrap();
    assert!(id_alloc::is_bare_token(derived));
}
