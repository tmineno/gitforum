//! v2.x CLI surface freeze (task `4w8hm98j`).
//!
//! One CLI-output-only regression test per `internal::commands::*` module.
//! Assertions read stdout/stderr/exit code or `git forum show` output —
//! never the storage shape at `refs/forum/threads/<id>`. These tests
//! must stay green across each Phase 2 cutover; storage-shape coverage
//! is versioned per phase under `tests/storage_v{2,3}_test.rs`.
//!
//! Coverage map: see `doc/internal/cli-coverage-audit.md`.

mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

fn bin() -> String {
    env!("CARGO_BIN_EXE_git-forum").to_string()
}

fn run(repo: &support::repo::TestRepo, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(repo.path())
        .args(args)
        .output()
        .expect("git-forum invocation failed")
}

fn run_ok(repo: &support::repo::TestRepo, args: &[&str]) -> Output {
    let out = run(repo, args);
    assert!(
        out.status.success(),
        "git-forum {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

fn extract_created_id(out: &Output) -> String {
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .strip_prefix("Created ")
        .unwrap_or(s.trim())
        .split_whitespace()
        .next()
        .expect("no thread id in `Created …` line")
        .to_string()
}

fn fresh_repo() -> support::repo::TestRepo {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    repo
}

fn show_stdout(repo: &support::repo::TestRepo, thread_id: &str) -> String {
    let out = run_ok(repo, &["show", thread_id]);
    String::from_utf8_lossy(&out.stdout).to_string()
}

// --- commands::thread_new -----------------------------------------------

#[test]
fn thread_new_visible_in_show() {
    let repo = fresh_repo();
    let out = run_ok(&repo, &["new", "issue", "Parser fails on empty input"]);
    let id = extract_created_id(&out);

    let show = show_stdout(&repo, &id);
    assert!(
        show.contains("Parser fails on empty input"),
        "show output missing thread title:\n{show}"
    );
}

// --- commands::shorthand_say --------------------------------------------

#[test]
fn comment_visible_in_show() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "Topic"]));

    run_ok(&repo, &["comment", &id, "I have a thought."]);

    let show = show_stdout(&repo, &id);
    assert!(
        show.contains("I have a thought."),
        "show output missing comment body:\n{show}"
    );
}

// --- commands::state ----------------------------------------------------

#[test]
fn close_visible_in_show() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "To be closed"]));

    // `close` for an execution-lifecycle issue maps to `done`
    // (SPEC-2.0 §9.3 lifecycle-aware shorthand).
    let close = run_ok(&repo, &["close", &id]);
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        close_stdout.contains(&id) && close_stdout.contains("done"),
        "close output missing `<id> -> done` line:\n{close_stdout}"
    );

    let show = show_stdout(&repo, &id);
    assert!(
        show.contains("**status:**") && show.contains("done"),
        "show output missing done status:\n{show}"
    );
}

// --- commands::bulk -----------------------------------------------------

#[test]
fn state_bulk_summary_visible() {
    let repo = fresh_repo();
    let a = extract_created_id(&run_ok(&repo, &["new", "issue", "Bulk A"]));
    let b = extract_created_id(&run_ok(&repo, &["new", "issue", "Bulk B"]));

    // `state bulk` takes thread ids positionally after `--to <state>`.
    let bulk = run_ok(&repo, &["state", "bulk", "--to", "closed", &a, &b]);
    let stdout = String::from_utf8_lossy(&bulk.stdout);
    assert!(
        stdout.contains(&a) && stdout.contains(&b),
        "bulk summary missing both thread ids:\n{stdout}"
    );
}

// --- commands::node_bulk ------------------------------------------------

#[test]
fn node_bulk_resolve_visible_in_show() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "With actions"]));

    // `action` stdout: first line is `Added action <node_id>`.
    let action_out = run_ok(&repo, &["action", &id, "Implement parser"]);
    let action_stdout = String::from_utf8_lossy(&action_out.stdout);
    let node_id = action_stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Added action "))
        .map(|s| s.trim().to_string())
        .expect("action stdout missing `Added action <id>` first line");

    run_ok(&repo, &["resolve", &id, &node_id]);

    let show = show_stdout(&repo, &id);
    assert!(
        show.contains("Implement parser"),
        "resolved action body missing from show:\n{show}"
    );
    // `show` lists open actions only; once resolved the action drops out
    // of the open list (advisory line "open actions: N" decrements).
    assert!(
        !show.contains("**open actions:** 1"),
        "show should no longer list this action as open:\n{show}"
    );
}

// --- commands::revise ---------------------------------------------------

#[test]
fn revise_body_visible_in_show() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(
        &repo,
        &["new", "issue", "Revise me", "--body", "first body"],
    ));

    run_ok(&repo, &["revise", &id, "--body", "second body"]);

    let show = show_stdout(&repo, &id);
    assert!(
        show.contains("second body"),
        "revised body missing from show:\n{show}"
    );
    assert!(
        !show.contains("first body"),
        "old body should not be present in show output:\n{show}"
    );
}

// --- commands::migrate (CLI surface only) -------------------------------
//
// SPEC-3.0 §8.1: `git forum migrate --to 3.0` is the only accepted
// invocation in v3.0.0. Bare `git forum migrate` (no `--to`) and
// unsupported targets (`--to 99.0`) MUST be rejected at the CLI layer
// with an actionable message. Body coverage (the actual walk + report)
// lives in `tests/migrate_validity_test.rs` (task `9635buy0` step 7).

#[test]
fn migrate_to_3_0_is_accepted_by_cli() {
    let repo = fresh_repo();
    // No legacy refs: the run is a no-op, but the CLI must accept the
    // invocation (exit 0, no clap error).
    let out = run(&repo, &["migrate", "--to", "3.0"]);
    assert!(
        out.status.success(),
        "git-forum migrate --to 3.0 should succeed on a fresh repo:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn migrate_rejects_unsupported_to_value() {
    let repo = fresh_repo();
    let out = run(&repo, &["migrate", "--to", "99.0"]);
    assert!(
        !out.status.success(),
        "git-forum migrate --to 99.0 must be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("3.0"),
        "rejection should point the user at `--to 3.0`:\n{stderr}"
    );
}

#[test]
fn migrate_without_to_is_rejected() {
    let repo = fresh_repo();
    let out = run(&repo, &["migrate"]);
    assert!(
        !out.status.success(),
        "bare `git forum migrate` must be rejected; --to is required"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--to"),
        "rejection should mention the missing --to argument:\n{stderr}"
    );
}
