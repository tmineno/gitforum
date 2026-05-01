//! Integration tests for Track G — Advisory layer.
//!
//! Acceptance criteria covered (see thread @fx5m13qu):
//! - `show --tree` lists only `--rel implements` children, never other relations.
//! - `brief` on a thread linked to others reports counts/relations only —
//!   never linked threads' titles, IDs, or states.
//! - `verify` surfaces a linked-thread advisory when the linked thread is not
//!   yet `done`, but does not change the verification result.
//! - `doctor` surfaces "parent done but children open" advisories.
//! - The SQLite reverse-link index resolves incoming `implements` queries.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::brief;
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::doctor;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::evidence;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::reindex;
use git_forum::internal::show;
use git_forum::internal::state_change::{self, StateChangeOptions};
use git_forum::internal::thread;
use git_forum::internal::verify;

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

fn make_thread(git: &GitOps, kind: ThreadKind, title: &str) -> String {
    create::create_thread(git, kind, title, None, "human/alice", &fixed_clock()).unwrap()
}

fn link(git: &GitOps, from: &str, to: &str, rel: &str) {
    evidence::add_thread_link(git, from, to, rel, "human/alice", &fixed_clock()).unwrap();
}

fn build_index(git: &GitOps, paths: &RepoPaths) {
    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(git, &db_path).unwrap();
}

fn open_index(paths: &RepoPaths) -> rusqlite::Connection {
    let db_path = paths.git_forum.join("index.db");
    index::open_db(&db_path).unwrap()
}

/// Walk a thread to its terminal `done` state along the shortest valid path.
///
/// Lifecycles vary in how many intermediate states stand between the initial
/// state and `done` (proposal: draft → open → done; execution: open → done;
/// record: open → done). The state machine guards reject multi-hop calls, so
/// this helper steps through intermediate states one at a time.
fn drive_to_done(git: &GitOps, policy: &Policy, thread_id: &str) {
    use git_forum::internal::event;
    loop {
        let state = thread::replay_thread(git, thread_id).unwrap();
        if event::normalize_state_name(&state.status) == "done" {
            break;
        }
        let lifecycle = state.lifecycle();
        let path = event::find_path(lifecycle, &state.status, "done")
            .unwrap_or_else(|| panic!("no path to done from {} for {:?}", state.status, lifecycle));
        let next = path.first().expect("path is empty but state != done");
        state_change::change_state(
            git,
            thread_id,
            next,
            &[],
            "human/alice",
            &fixed_clock(),
            policy,
            StateChangeOptions::default(),
        )
        .unwrap();
    }
}

// ---- show --tree ----

#[test]
fn show_tree_lists_only_implements_children_not_other_relations() {
    let (_repo, git, paths) = setup();

    let parent = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    let impl_child = make_thread(&git, ThreadKind::Task, "Implementing task");
    let related_sibling = make_thread(&git, ThreadKind::Dec, "Related decision");

    // One `implements`, one `relates-to`. The `--tree` advisory must show only
    // the `implements` child per SPEC-2.0 §2.1 / §B.4.
    link(&git, &impl_child, &parent, "implements");
    link(&git, &related_sibling, &parent, "relates-to");

    build_index(&git, &paths);

    let conn = open_index(&paths);
    let rows = index::find_incoming_links(&conn, &parent, Some("implements")).unwrap();
    let from_ids: Vec<&str> = rows.iter().map(|r| r.from_thread_id.as_str()).collect();
    assert_eq!(from_ids, vec![impl_child.as_str()]);

    let parent_state = thread::replay_thread(&git, &parent).unwrap();
    let mut children = Vec::new();
    for row in &rows {
        let s = thread::replay_thread(&git, &row.from_thread_id).unwrap();
        children.push(show::TreeChild {
            id: s.id.clone(),
            title: s.title.clone(),
            lifecycle_label: s.lifecycle().as_str().to_string(),
            status: s.status.clone(),
        });
    }

    let out = show::render_tree(&parent_state, &children);

    // The `implements` child appears.
    assert!(
        out.contains(&impl_child),
        "expected implements child in tree output:\n{out}"
    );
    assert!(out.contains("Implementing task"));
    // The `relates-to` sibling MUST NOT appear in the tree.
    assert!(
        !out.contains(&related_sibling),
        "relates-to sibling leaked into --tree output:\n{out}"
    );
    assert!(!out.contains("Related decision"));
}

#[test]
fn show_tree_advisory_does_not_recurse() {
    // Acceptance: `--tree` is one hop only. A grandchild that implements the
    // child must not appear under the parent's tree output.
    let (_repo, git, paths) = setup();

    let parent = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    let child = make_thread(&git, ThreadKind::Task, "Direct child");
    let grandchild = make_thread(&git, ThreadKind::Task, "Grandchild");

    link(&git, &child, &parent, "implements");
    link(&git, &grandchild, &child, "implements");

    build_index(&git, &paths);

    let conn = open_index(&paths);
    let rows = index::find_incoming_links(&conn, &parent, Some("implements")).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].from_thread_id, child);
}

// ---- brief command ----

#[test]
fn brief_does_not_name_linked_threads_titles_or_states() {
    // Acceptance: `brief` on a thread with linked siblings produces output
    // that does not name the linked threads' titles or states.
    let (_repo, git, paths) = setup();

    let subject = make_thread(&git, ThreadKind::Rfc, "Subject RFC");
    let sibling_a = make_thread(
        &git,
        ThreadKind::Task,
        "Highly distinctive sibling title AAA",
    );
    let sibling_b = make_thread(
        &git,
        ThreadKind::Dec,
        "Another distinctive sibling title BBB",
    );

    // Subject -> sibling_a (implements), sibling_b -> subject (relates-to).
    link(&git, &subject, &sibling_a, "implements");
    link(&git, &sibling_b, &subject, "relates-to");

    build_index(&git, &paths);

    let state = thread::replay_thread(&git, &subject).unwrap();
    let conn = open_index(&paths);
    let mut counts = brief::IncomingLinkCounts::default();
    for row in index::find_incoming_links(&conn, &subject, None).unwrap() {
        *counts.by_rel.entry(row.rel).or_insert(0) += 1;
    }

    let out = brief::render_plaintext(&state, &counts);

    // Counts and relations are visible.
    assert!(
        out.contains("in=1 relates-to"),
        "expected incoming relates-to count:\n{out}"
    );
    assert!(
        out.contains("out=1 implements"),
        "expected outgoing implements count:\n{out}"
    );

    // Linked threads' IDs, titles, or any other identifying detail MUST NOT appear.
    assert!(
        !out.contains(&sibling_a),
        "linked sibling_a id leaked:\n{out}"
    );
    assert!(
        !out.contains(&sibling_b),
        "linked sibling_b id leaked:\n{out}"
    );
    assert!(
        !out.contains("Highly distinctive sibling title AAA"),
        "linked sibling_a title leaked:\n{out}"
    );
    assert!(
        !out.contains("Another distinctive sibling title BBB"),
        "linked sibling_b title leaked:\n{out}"
    );
}

#[test]
fn brief_json_emits_stable_v1_schema_fields() {
    let (_repo, git, paths) = setup();
    let subject = make_thread(&git, ThreadKind::Rfc, "Subject RFC");
    let target = make_thread(&git, ThreadKind::Task, "Target task");
    link(&git, &subject, &target, "implements");
    build_index(&git, &paths);

    let state = thread::replay_thread(&git, &subject).unwrap();
    let counts = brief::IncomingLinkCounts::default();
    let payload = brief::build_json(&state, &counts);
    let serialized = serde_json::to_value(&payload).unwrap();

    // Required fields per RFC-5wf2v8hv acceptance criteria.
    for field in [
        "id",
        "title",
        "lifecycle",
        "tags",
        "status",
        "created_at",
        "created_by",
        "branch",
        "links_in",
        "links_out",
        "node_counts",
        "evidence_count",
        "latest_summary",
    ] {
        assert!(
            serialized.get(field).is_some(),
            "missing JSON field `{field}`: {serialized}"
        );
    }
}

// ---- verify advisory ----

#[test]
fn verify_surfaces_linked_thread_advisory_when_target_not_done() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let task = make_thread(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    // task implements an RFC that is still in `open` (not `done`).
    link(&git, &task, &rfc, "implements");

    let report = verify::verify_thread(&git, &task, &policy).unwrap();

    // Per SPEC-2.0 §9.4, the verify advisory is informational. The verify
    // result for the named thread is decided by single-thread guards only —
    // not by whether the linked RFC is `done`.
    assert!(
        !report.linked_advisories.is_empty(),
        "expected at least one linked-thread advisory"
    );
    let adv = &report.linked_advisories[0];
    assert_eq!(adv.linked_thread_id, rfc);
    assert!(adv.message.contains("not yet `done`"));
}

#[test]
fn verify_omits_advisory_when_linked_thread_is_done() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let task = make_thread(&git, ThreadKind::Task, "Implementing task");
    let rfc = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    link(&git, &task, &rfc, "implements");

    // Drive the RFC to `done` so the advisory must be silent.
    drive_to_done(&git, &policy, &rfc);

    let report = verify::verify_thread(&git, &task, &policy).unwrap();
    assert!(
        report.linked_advisories.is_empty(),
        "advisory should not fire when linked thread is `done`: {:?}",
        report.linked_advisories
    );
}

// ---- doctor advisory ----

#[test]
fn doctor_surfaces_open_implementing_children_under_done_parent() {
    let (_repo, git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();

    let parent = make_thread(&git, ThreadKind::Rfc, "Parent RFC");
    let child_open = make_thread(&git, ThreadKind::Task, "Still working");
    let child_done = make_thread(&git, ThreadKind::Task, "Already done");
    link(&git, &child_open, &parent, "implements");
    link(&git, &child_done, &parent, "implements");

    // parent → done; child_done → done; child_open stays in flight.
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
    make_thread(&git, ThreadKind::Rfc, "Lonely RFC");
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.advisories.is_empty(),
        "no advisories expected: {:?}",
        report.advisories
    );
}
