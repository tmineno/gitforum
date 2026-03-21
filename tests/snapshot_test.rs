mod support;

use chrono::TimeZone;
use git_forum::internal::event::{Event, EventType, NodeType, ThreadKind};
use git_forum::internal::node::Node;
use git_forum::internal::show;
use git_forum::internal::thread::{NodeLookup, ThreadLink, ThreadState};

const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots");

/// Compare output against a golden snapshot file.
///
/// If the environment variable `UPDATE_SNAPSHOTS=1` is set the file is written
/// instead of compared, so you can regenerate all snapshots in one pass:
///
///     UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test
fn assert_snapshot(name: &str, actual: &str) {
    let path = format!("{SNAPSHOT_DIR}/{name}.snap");
    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("snapshot {path} not found ({e}). Run with UPDATE_SNAPSHOTS=1 to create it.")
    });
    assert_eq!(actual, expected, "snapshot mismatch for {name} ({path})");
}

fn fixed_time() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn base_state() -> ThreadState {
    let t = fixed_time();
    ThreadState {
        id: "RFC-0001".into(),
        kind: ThreadKind::Rfc,
        title: "Test RFC".into(),
        body: Some("Initial thread body.".into()),
        branch: None,
        status: "draft".into(),
        created_at: t,
        created_by: "human/alice".into(),
        events: vec![Event {
            event_id: "evt-0001".into(),
            thread_id: "RFC-0001".into(),
            event_type: EventType::Create,
            created_at: t,
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: Some("Test RFC".into()),
            kind: Some(ThreadKind::Rfc),
            body: None,
            node_type: None,
            target_node_id: None,
            new_state: None,
            approvals: vec![],
            evidence: None,
            link_rel: None,
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
        }],
        nodes: vec![],
        evidence_items: vec![],
        links: vec![],
        body_revision_count: 0,
        incorporated_node_ids: vec![],
    }
}

fn rich_state() -> ThreadState {
    let t = fixed_time();
    let t2 = t + chrono::Duration::hours(1);
    let t3 = t + chrono::Duration::hours(2);
    let mut state = base_state();
    state.status = "under-review".into();
    state.branch = Some("feat/solver".into());
    state.nodes = vec![
        Node {
            node_id: "node-0001".into(),
            node_type: NodeType::Objection,
            body: "Benchmarks are missing.".into(),
            actor: "ai/reviewer".into(),
            created_at: t2,
            resolved: false,
            retracted: false,
            incorporated: false,
            reply_to: None,
        },
        Node {
            node_id: "node-0002".into(),
            node_type: NodeType::Summary,
            body: "Direction is sound; migration evidence needed.".into(),
            actor: "human/alice".into(),
            created_at: t3,
            resolved: false,
            retracted: false,
            incorporated: false,
            reply_to: None,
        },
    ];
    state.events.push(Event {
        event_id: "evt-0002".into(),
        thread_id: "RFC-0001".into(),
        event_type: EventType::Say,
        created_at: t2,
        actor: "ai/reviewer".into(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some("Benchmarks are missing.".into()),
        node_type: Some(NodeType::Objection),
        target_node_id: Some("node-0001".into()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        branch: None,
        incorporated_node_ids: vec![],
        reply_to: None,
    });
    state.events.push(Event {
        event_id: "evt-0003".into(),
        thread_id: "RFC-0001".into(),
        event_type: EventType::Say,
        created_at: t3,
        actor: "human/alice".into(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some("Direction is sound; migration evidence needed.".into()),
        node_type: Some(NodeType::Summary),
        target_node_id: Some("node-0002".into()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        branch: None,
        incorporated_node_ids: vec![],
        reply_to: None,
    });
    state
}

// ---- show ----

#[test]
fn show_basic_rfc() {
    let state = base_state();
    let out = show::render_show(&state, false);
    assert_snapshot("show_basic_rfc", &out);
}

#[test]
fn show_rich_rfc() {
    let state = rich_state();
    let out = show::render_show(&state, false);
    assert_snapshot("show_rich_rfc", &out);
}

// ---- node show ----

#[test]
fn node_show_question() {
    let t = fixed_time();
    let lookup = NodeLookup {
        thread_id: "RFC-0001".into(),
        thread_title: "Test RFC".into(),
        thread_kind: ThreadKind::Rfc,
        node: Node {
            node_id: "node-0001".into(),
            node_type: NodeType::Question,
            body: "What is the migration plan?".into(),
            actor: "ai/reviewer".into(),
            created_at: t,
            resolved: false,
            retracted: false,
            incorporated: false,
            reply_to: None,
        },
        links: vec![ThreadLink {
            target_thread_id: "ISSUE-0001".into(),
            rel: "implements".into(),
        }],
        events: vec![Event {
            event_id: "evt-0010".into(),
            thread_id: "RFC-0001".into(),
            event_type: EventType::Say,
            created_at: t,
            actor: "ai/reviewer".into(),
            base_rev: None,
            parents: vec![],
            title: None,
            kind: None,
            body: Some("What is the migration plan?".into()),
            node_type: Some(NodeType::Question),
            target_node_id: Some("node-0001".into()),
            new_state: None,
            approvals: vec![],
            evidence: None,
            link_rel: None,
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
        }],
    };
    let out = show::render_node_show(&lookup);
    assert_snapshot("node_show_question", &out);
}

// ---- ls ----

#[test]
fn ls_empty() {
    let out = show::render_ls(&[]);
    assert_snapshot("ls_empty", &out);
}

#[test]
fn ls_two_threads() {
    let s1 = base_state();
    let mut s2 = base_state();
    s2.id = "ISSUE-0001".into();
    s2.kind = ThreadKind::Issue;
    s2.title = "Implement trait backend".into();
    s2.status = "open".into();
    s2.branch = Some("feat/parser".into());
    let out = show::render_ls(&[&s1, &s2]);
    assert_snapshot("ls_two_threads", &out);
}

// ---- status ----

#[test]
fn status_with_objection() {
    let state = rich_state();
    let out = show::render_status(&state);
    assert_snapshot("status_with_objection", &out);
}

#[test]
fn status_all_clean() {
    let state = base_state();
    let out = show::render_status_all(&[&state]);
    assert_snapshot("status_all_clean", &out);
}
