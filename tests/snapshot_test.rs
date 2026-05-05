mod support;

use chrono::TimeZone;
use git_forum::internal::commands::ls;
use git_forum::internal::commands::show;
use git_forum::internal::node::{Node, NodeKind};
use git_forum::internal::policy::Lifecycle;
use git_forum::internal::thread::{NodeLookup, ThreadKind, ThreadLink, ThreadState, ThreadStatus};

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
        id: "RFC-a1b2c3d4".into(),
        kind: ThreadKind::Rfc,
        // Phase 2c: keep lifecycle aligned with kind so the snapshot's
        // proposal-flavored state-machine transitions don't pick up
        // execution lifecycle's `working` state by default.
        lifecycle: Lifecycle::Proposal,
        title: "Test RFC".into(),
        body: Some("Initial thread body.".into()),
        status: ThreadStatus::Draft,
        created_at: t,
        created_by: "human/alice".into(),
        updated_at: t,
        ..ThreadState::default()
    }
}

fn rich_state() -> ThreadState {
    let t = fixed_time();
    let t2 = t + chrono::Duration::hours(1);
    let t3 = t + chrono::Duration::hours(2);
    let mut state = base_state();
    // Lenient parse so 1.x-flavored fixture data still produces a valid 2.0 status.
    state.status = ThreadStatus::parse_lenient("under-review").unwrap();
    state.branch = Some("feat/solver".into());
    state.updated_at = t3;
    state.nodes = vec![
        Node {
            node_id: "node-0001".into(),
            node_type: NodeKind::Objection,
            body: "Benchmarks are missing.".into(),
            actor: "ai/reviewer".into(),
            created_at: t2,
            ..Node::default()
        },
        Node {
            node_id: "node-0002".into(),
            node_type: NodeKind::Comment,
            legacy_subtype: Some("summary".into()),
            body: "Direction is sound; migration evidence needed.".into(),
            actor: "human/alice".into(),
            created_at: t3,
            ..Node::default()
        },
    ];
    state
}

// ---- show ----

#[test]
fn show_basic_rfc() {
    let state = base_state();
    let out = show::render_show(&state, &show::ShowOptions::default());
    assert_snapshot("show_basic_rfc", &out);
}

#[test]
fn show_rich_rfc() {
    let state = rich_state();
    let out = show::render_show(&state, &show::ShowOptions::default());
    assert_snapshot("show_rich_rfc", &out);
}

#[test]
fn show_rich_rfc_compact() {
    let state = rich_state();
    let out = show::render_show(
        &state,
        &show::ShowOptions {
            compact: true,
            ..show::ShowOptions::default()
        },
    );
    assert_snapshot("show_rich_rfc_compact", &out);
}

#[test]
fn show_rich_rfc_no_timeline() {
    let state = rich_state();
    let out = show::render_show(
        &state,
        &show::ShowOptions {
            no_timeline: true,
            ..show::ShowOptions::default()
        },
    );
    assert_snapshot("show_rich_rfc_no_timeline", &out);
}

#[test]
fn show_rich_rfc_what_next() {
    let state = rich_state();
    let policy = git_forum::internal::policy::Policy::default();
    let out = show::render_show(
        &state,
        &show::ShowOptions {
            mode: show::ShowMode::WhatNext,
            policy: Some(policy),
            ..show::ShowOptions::default()
        },
    );
    assert_snapshot("show_rich_rfc_what_next", &out);
}

// ---- node show ----

#[test]
fn node_show_question() {
    let t = fixed_time();
    let lookup = NodeLookup {
        thread_id: "RFC-a1b2c3d4".into(),
        thread_title: "Test RFC".into(),
        thread_kind: ThreadKind::Rfc,
        // Phase 2b: NodeLookup carries the parent's lifecycle / tags so
        // `node show` can display the canonical 2.0 axes.
        thread_lifecycle: Lifecycle::Proposal,
        thread_tags: vec!["cross-cutting".into()],
        node: Node {
            node_id: "node-0001".into(),
            node_type: NodeKind::Comment,
            legacy_subtype: Some("question".into()),
            body: "What is the migration plan?".into(),
            actor: "ai/reviewer".into(),
            created_at: t,
            ..Node::default()
        },
        links: vec![ThreadLink {
            target_thread_id: "ASK-e5f6a7b8".into(),
            rel: "implements".into(),
        }],
    };
    let out = show::render_node_show(&lookup, &show::ShowOptions::default());
    assert_snapshot("node_show_question", &out);
}

// ---- ls ----

#[test]
fn ls_empty() {
    let out = ls::render_ls(&[]);
    assert_snapshot("ls_empty", &out);
}

#[test]
fn ls_two_threads() {
    let s1 = base_state();
    let mut s2 = base_state();
    s2.id = "ASK-e5f6a7b8".into();
    s2.kind = ThreadKind::Issue;
    // Phase 2b: keep lifecycle aligned when changing kind on a fixture.
    s2.lifecycle = Lifecycle::Execution;
    s2.tags = vec!["bug".into()];
    s2.title = "Implement trait backend".into();
    s2.status = ThreadStatus::Open;
    s2.branch = Some("feat/parser".into());
    let out = ls::render_ls(&[&s1, &s2]);
    assert_snapshot("ls_two_threads", &out);
}

// ---- status ----

#[test]
fn status_with_objection() {
    let state = rich_state();
    let out = show::render_show(
        &state,
        &show::ShowOptions {
            mode: show::ShowMode::Status,
            ..show::ShowOptions::default()
        },
    );
    assert_snapshot("status_with_objection", &out);
}
