//! Module integration tests for `src/internal/brief.rs` —
//! the read-only single-thread digest used by the `brief` command
//! (test-policy.md category 1, RFC-5wf2v8hv).

mod support;

use git_forum::internal::brief;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::index;
use git_forum::internal::thread;

use support::forum::{build_index, link_thread as link, make_thread, open_index, setup};

#[test]
fn brief_does_not_name_linked_threads_titles_or_states() {
    // RFC-5wf2v8hv acceptance: `brief` on a thread with linked siblings
    // produces output that does not name the linked threads' titles or
    // states.
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

    assert!(
        out.contains("in=1 relates-to"),
        "expected incoming relates-to count:\n{out}"
    );
    assert!(
        out.contains("out=1 implements"),
        "expected outgoing implements count:\n{out}"
    );

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
