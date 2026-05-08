//! Ticket `ah5gepry`: audit user-facing vocabulary across `ls`,
//! `show`, `status`, top-level `--help`, and `--help-llm`.
//!
//! These tests pin the canonical 3.0 nouns (`category`, `tags`,
//! `lifecycle` label) and document `--kind` / kind preset as a
//! compatibility alias. They are intentionally narrow — they assert on
//! help text, not on storage shape — so a vocabulary regression flips
//! one of these tests instead of leaking through to a user.

use std::process::Command;

fn run_help(args: &[&str]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .args(args)
        .output()
        .expect("failed to run git-forum");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn ls_help_uses_canonical_lifecycle_vocabulary() {
    let (stdout, _stderr, ok) = run_help(&["ls", "--help"]);
    assert!(ok, "ls --help must succeed");

    // Canonical 3.0 axes appear in `ls`'s self-description.
    assert!(
        stdout.contains("lifecycle"),
        "ls --help should reference the canonical LIFECYCLE axis, got:\n{stdout}"
    );
    // Document `--kind` as a compatibility alias rather than the
    // primary 3.0 axis.
    assert!(
        stdout.to_lowercase().contains("legacy") || stdout.to_lowercase().contains("compatibility"),
        "ls --help should mark `--kind` as a legacy compatibility alias, got:\n{stdout}"
    );
}

#[test]
fn shortlog_help_marks_kind_as_legacy_alias() {
    let (stdout, _stderr, ok) = run_help(&["shortlog", "--help"]);
    assert!(ok, "shortlog --help must succeed");
    assert!(
        stdout.to_lowercase().contains("legacy") || stdout.to_lowercase().contains("compatibility"),
        "shortlog --help should mark `--kind` as a legacy alias, got:\n{stdout}"
    );
}

#[test]
fn top_level_grouped_help_uses_lifecycle_for_ls_description() {
    let (stdout, _stderr, ok) = run_help(&["--help"]);
    assert!(ok, "git-forum --help must succeed");
    assert!(
        stdout.contains("ls") && stdout.contains("lifecycle"),
        "GROUPED_HELP `ls` line should mention the lifecycle axis, got:\n{stdout}"
    );
    // The `state shorthands` block describes lifecycle-aware behaviour.
    assert!(
        stdout.contains("lifecycle"),
        "GROUPED_HELP should keep the lifecycle terminology for state shorthands, got:\n{stdout}"
    );
}

#[test]
fn help_llm_anchors_3_0_vocabulary_section() {
    let (stdout, _stderr, ok) = run_help(&["--help-llm"]);
    assert!(ok, "git-forum --help-llm must succeed");

    // `--help-llm` is the verbatim MANUAL.md. The 3.0 vocabulary
    // section anchors the canonical nouns.
    assert!(
        stdout.contains("3.0 vocabulary"),
        "--help-llm should expose the `3.0 vocabulary` section header"
    );
    assert!(
        stdout.contains("category") && stdout.contains("tags"),
        "--help-llm should describe storage in terms of `category` + `tags`"
    );
    // Kind presets are described as 1.x/legacy carryovers.
    assert!(
        stdout.contains("kind preset") || stdout.contains("kind presets"),
        "--help-llm should describe the kind presets explicitly"
    );
    // Manual must reflect the actual `thread new` CLI surface.
    assert!(
        stdout.contains("--lifecycle"),
        "--help-llm `thread new` synopsis must match the actual CLI flag (`--lifecycle`); \
         ticket `ah5gepry` aligns the manual to the binary."
    );
}
