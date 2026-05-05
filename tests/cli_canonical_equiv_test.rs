/// Equivalence tests: canonical forms produce the same events as shorthand forms.
/// Validates RFC-0024 — generic-first canonical CLI policy.
mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::node::NodeKind;
use git_forum::internal::thread;

fn bin() -> String {
    env!("CARGO_BIN_EXE_git-forum").to_string()
}

fn run(repo_path: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .current_dir(repo_path)
        .args(args)
        .output()
        .expect("command failed to execute")
}

fn extract_created_id(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .strip_prefix("Created ")
        .unwrap_or(stdout.trim())
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn setup_issue(repo_path: &std::path::Path) -> String {
    let out = run(repo_path, &["new", "ask", "Test issue"]);
    assert!(out.status.success(), "failed to create issue");
    extract_created_id(&out)
}

fn setup_rfc(repo_path: &std::path::Path) -> String {
    let out = run(
        repo_path,
        &[
            "new",
            "rfc",
            "Test RFC",
            "--body",
            "## Goal\nTest.\n## Non-goals\nNone.\n## Context\nTest.\n## Proposal\nTest.",
        ],
    );
    assert!(out.status.success(), "failed to create rfc");
    extract_created_id(&out)
}

fn replay(
    repo_path: &std::path::Path,
    thread_id: &str,
) -> git_forum::internal::thread::ThreadState {
    let git = GitOps::new(repo_path.to_path_buf());
    thread::replay_thread(&git, thread_id).unwrap()
}

// --- Node equivalence tests ---
//
// `claim_shorthand_equals_node_add_claim` and
// `question_shorthand_equals_node_add_question` were removed at
// Phase 2 slot 2 (RFC `7ymtc4b2`). The deprecated rhetorical
// shorthands (`claim`, `question`, `summary`, `risk`, `review`) are
// no longer in the CLI surface; SPEC-3.0 §2.2 / ADR-006 keeps only
// the four canonical NodeKinds (Comment, Approval, Objection, Action).

#[test]
fn objection_shorthand_equals_node_add_objection() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_rfc(repo_a.path());
    let out = run(repo_a.path(), &["objection", &id_a, "Missing benchmarks"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_rfc(repo_b.path());
    let out = run(
        repo_b.path(),
        &[
            "node",
            "add",
            &id_b,
            "--type",
            "objection",
            "Missing benchmarks",
        ],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.nodes[0].record.kind, NodeKind::Objection);
    assert_eq!(state_b.nodes[0].record.kind, NodeKind::Objection);
    assert_eq!(state_a.nodes[0].body, state_b.nodes[0].body);
}

// --- State equivalence tests ---

#[test]
fn close_shorthand_equals_state_closed() {
    // Shorthand: git forum close <ID>
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["close", &id_a]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    // Canonical: git forum state <ID> closed
    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", &id_b, "closed"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.status, "done");
    assert_eq!(state_b.status, "done");
}

#[test]
fn pend_shorthand_equals_state_pending() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["pend", &id_a]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", &id_b, "pending"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.status, "working");
    assert_eq!(state_b.status, "working");
}

#[test]
fn reject_shorthand_equals_state_rejected() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["reject", &id_a]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", &id_b, "rejected"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.status, "rejected");
    assert_eq!(state_b.status, "rejected");
}

#[test]
fn propose_shorthand_equals_state_proposed() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_rfc(repo_a.path());
    let out = run(repo_a.path(), &["propose", &id_a]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_rfc(repo_b.path());
    let out = run(repo_b.path(), &["state", &id_b, "proposed"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.status, "open");
    assert_eq!(state_b.status, "open");
}

#[test]
fn accept_shorthand_equals_state_accepted() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    let id_a = setup_rfc(repo_a.path());
    // Move to under-review first
    let out = run(repo_a.path(), &["state", &id_a, "proposed"]);
    assert!(out.status.success());
    let out = run(repo_a.path(), &["state", &id_a, "under-review"]);
    assert!(out.status.success());
    // SPEC-3.0 §2.2 / ADR-006: the rhetorical `summary` shorthand
    // collapsed onto `comment`. The acceptance guard keys off the
    // canonical node, not the rhetorical label.
    let out = run(repo_a.path(), &["comment", &id_a, "Looks good"]);
    assert!(out.status.success());
    let out = run(
        repo_a.path(),
        &["accept", &id_a, "--approve", "human/alice"],
    );
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), &id_a);

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    let id_b = setup_rfc(repo_b.path());
    let out = run(repo_b.path(), &["state", &id_b, "proposed"]);
    assert!(out.status.success());
    let out = run(repo_b.path(), &["state", &id_b, "under-review"]);
    assert!(out.status.success());
    let out = run(repo_b.path(), &["comment", &id_b, "Looks good"]);
    assert!(out.status.success());
    let out = run(
        repo_b.path(),
        &["state", &id_b, "accepted", "--approve", "human/alice"],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), &id_b);

    assert_eq!(state_a.status, "done");
    assert_eq!(state_b.status, "done");
}
