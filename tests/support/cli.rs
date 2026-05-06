//! Shared `git-forum` subprocess helpers.
//!
//! Centralizes the per-file copies of `bin`/`run`/`run_ok`/
//! `extract_created_id`/`fresh_repo`/`make_thread_via_cli` that
//! previously lived in `tests/cli_*_test.rs` and
//! `tests/storage_v3_test.rs`. Each `tests/*_test.rs` compiles
//! `support` independently, so unused helpers must not warn.

#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

use super::repo::TestRepo;

/// Path to the compiled `git-forum` binary under test.
pub fn bin() -> String {
    env!("CARGO_BIN_EXE_git-forum").to_string()
}

/// Run `git-forum <args>` in `repo_path`. Returns the raw `Output`
/// without asserting success — callers that need failure paths
/// inspect `out.status` themselves.
pub fn run(repo_path: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(repo_path)
        .args(args)
        .output()
        .expect("git-forum invocation failed")
}

/// Run `git-forum <args>` and assert success. On failure, panic with
/// the full stdout/stderr so test diagnostics survive the move.
pub fn run_ok(repo_path: &Path, args: &[&str]) -> Output {
    let out = run(repo_path, args);
    assert!(
        out.status.success(),
        "git-forum {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

/// Parse the thread id from a `Created <id> ...` stdout line.
/// Tolerates trailing hint suffixes after the id.
pub fn extract_created_id(out: &Output) -> String {
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .strip_prefix("Created ")
        .unwrap_or(s.trim())
        .split_whitespace()
        .next()
        .expect("no thread id in `Created …` line")
        .to_string()
}

/// Fresh isolated `TestRepo` with `.forum/` initialized via
/// `init::init_forum`. Use for CLI tests that drive subcommands.
pub fn fresh_repo() -> TestRepo {
    let repo = TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    repo
}

/// Create a thread via `git-forum new <kind> <title> --body <body>`
/// and return the new thread id. Replaces the legacy
/// `create::create_thread` fixture path now that task `1v400j3l`
/// forbids non-migrate code paths from consuming legacy event chains.
pub fn make_thread_via_cli(repo_path: &Path, kind: &str, title: &str, body: &str) -> String {
    let output = Command::new(bin())
        .current_dir(repo_path)
        .args(["new", kind, title, "--body", body])
        .output()
        .expect("failed to run");
    assert!(
        output.status.success(),
        "make_thread_via_cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    extract_created_id(&output)
}
