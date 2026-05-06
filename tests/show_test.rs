//! Module integration tests for `src/internal/commands/show.rs`
//! rendering (test-policy.md category 1).
//!
//! task `913c4s9v`: rewritten to use
//! the snapshot fixtures from `support::forum`. The v2 test paths
//! (write_ops::say_node, evidence::add_thread_link, reindex+index for
//! the `--tree` advisory) targeted modules that this commit deletes;
//! tests for those features either move to per-feature snapshot tests
//! (snapshot_store_test, snapshot_test) or, for v2-only behaviors
//! (objection / summary / claim node rendering, `--tree` via index),
//! are removed pending the v3.1 search/index reintroduction decision
//! tracked from RFC `7ymtc4b2` / task `913c4s9v`.

mod support;

use git_forum::internal::commands::show;
use git_forum::internal::thread;

use support::forum::{make_dec, make_rfc, make_task, setup};

#[test]
fn show_contains_all_required_fields() {
    let (_repo, git, _paths) = setup();
    let id = make_rfc(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    // task `913c4s9v`: timeline is now the snapshot ref's git log
    // (SPEC-3.0 §5.4). Load entries so the renderer fills the table
    // instead of emitting the "no snapshot ref loaded" placeholder.
    let timeline_entries =
        git_forum::internal::snapshot::history::read_log(&git, &format!("refs/forum/threads/{id}"))
            .ok();
    let out = show::render_show(
        &state,
        &show::ShowOptions {
            timeline_entries,
            ..show::ShowOptions::default()
        },
    );

    assert!(out.contains(&id), "missing thread id");
    assert!(out.contains("Test RFC"), "missing title");
    // SPEC-3.0 category=rfc → lifecycle=proposal display.
    assert!(out.contains("proposal"), "missing lifecycle");
    assert!(out.contains("draft"), "missing status");
    assert!(out.contains("human/alice"), "missing actor");
    assert!(out.contains("---"), "missing body separator");
    assert!(out.contains("2026-01-01T00:00:00Z"), "missing timestamp");
    assert!(out.contains("### timeline"), "missing timeline section");
    // SPEC-3.0 §5.4 timeline columns: date | sha | author | op | detail.
    assert!(
        out.contains("| date | sha | author | op | detail |"),
        "missing 3.0 timeline header"
    );
}

#[test]
fn show_replay_consistency() {
    let (_repo, git, _paths) = setup();
    let id = make_rfc(&git);
    let state1 = thread::replay_thread(&git, &id).unwrap();
    let state2 = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(
        show::render_show(&state1, &show::ShowOptions::default()),
        show::render_show(&state2, &show::ShowOptions::default())
    );
}

#[test]
fn show_dec_renders_record_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**lifecycle:** record"));
    assert!(output.contains("**status:**    open"));
}

#[test]
fn show_task_renders_execution_lifecycle() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**lifecycle:** execution"));
    assert!(output.contains("**status:**    open"));
}
