/// Equivalence tests: canonical forms produce the same events as shorthand forms.
/// Validates RFC-0024 — generic-first canonical CLI policy.
mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::NodeType;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
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

fn setup_issue(repo_path: &std::path::Path) {
    let out = run(repo_path, &["new", "issue", "Test issue"]);
    assert!(out.status.success(), "failed to create issue");
}

fn setup_rfc(repo_path: &std::path::Path) {
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
}

fn replay(
    repo_path: &std::path::Path,
    thread_id: &str,
) -> git_forum::internal::thread::ThreadState {
    let git = GitOps::new(repo_path.to_path_buf());
    thread::replay_thread(&git, thread_id).unwrap()
}

// --- Node equivalence tests ---

#[test]
fn claim_shorthand_equals_node_add_claim() {
    // Shorthand: git forum claim <ID> "body"
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_rfc(repo_a.path());
    let out = run(repo_a.path(), &["claim", "RFC-0001", "Shorthand claim"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "RFC-0001");

    // Canonical: git forum node add <ID> --type claim "body"
    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_rfc(repo_b.path());
    let out = run(
        repo_b.path(),
        &[
            "node",
            "add",
            "RFC-0001",
            "--type",
            "claim",
            "Shorthand claim",
        ],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "RFC-0001");

    assert_eq!(state_a.nodes.len(), state_b.nodes.len());
    assert_eq!(state_a.nodes[0].node_type, NodeType::Claim);
    assert_eq!(state_b.nodes[0].node_type, NodeType::Claim);
    assert_eq!(state_a.nodes[0].body, state_b.nodes[0].body);
}

#[test]
fn question_shorthand_equals_node_add_question() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_rfc(repo_a.path());
    let out = run(repo_a.path(), &["question", "RFC-0001", "Is this safe?"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "RFC-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_rfc(repo_b.path());
    let out = run(
        repo_b.path(),
        &[
            "node",
            "add",
            "RFC-0001",
            "--type",
            "question",
            "Is this safe?",
        ],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "RFC-0001");

    assert_eq!(state_a.nodes.len(), state_b.nodes.len());
    assert_eq!(state_a.nodes[0].node_type, NodeType::Question);
    assert_eq!(state_b.nodes[0].node_type, NodeType::Question);
    assert_eq!(state_a.nodes[0].body, state_b.nodes[0].body);
}

#[test]
fn objection_shorthand_equals_node_add_objection() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_rfc(repo_a.path());
    let out = run(
        repo_a.path(),
        &["objection", "RFC-0001", "Missing benchmarks"],
    );
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "RFC-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_rfc(repo_b.path());
    let out = run(
        repo_b.path(),
        &[
            "node",
            "add",
            "RFC-0001",
            "--type",
            "objection",
            "Missing benchmarks",
        ],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "RFC-0001");

    assert_eq!(state_a.nodes[0].node_type, NodeType::Objection);
    assert_eq!(state_b.nodes[0].node_type, NodeType::Objection);
    assert_eq!(state_a.nodes[0].body, state_b.nodes[0].body);
}

// --- State equivalence tests ---

#[test]
fn close_shorthand_equals_state_closed() {
    // Shorthand: git forum close <ID>
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["close", "ASK-0001"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "ASK-0001");

    // Canonical: git forum state <ID> closed
    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", "ASK-0001", "closed"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "ASK-0001");

    assert_eq!(state_a.status, "closed");
    assert_eq!(state_b.status, "closed");
}

#[test]
fn pend_shorthand_equals_state_pending() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["pend", "ASK-0001"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "ASK-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", "ASK-0001", "pending"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "ASK-0001");

    assert_eq!(state_a.status, "pending");
    assert_eq!(state_b.status, "pending");
}

#[test]
fn reject_shorthand_equals_state_rejected() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_issue(repo_a.path());
    let out = run(repo_a.path(), &["reject", "ASK-0001"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "ASK-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_issue(repo_b.path());
    let out = run(repo_b.path(), &["state", "ASK-0001", "rejected"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "ASK-0001");

    assert_eq!(state_a.status, "rejected");
    assert_eq!(state_b.status, "rejected");
}

#[test]
fn propose_shorthand_equals_state_proposed() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_rfc(repo_a.path());
    let out = run(repo_a.path(), &["propose", "RFC-0001"]);
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "RFC-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_rfc(repo_b.path());
    let out = run(repo_b.path(), &["state", "RFC-0001", "proposed"]);
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "RFC-0001");

    assert_eq!(state_a.status, "proposed");
    assert_eq!(state_b.status, "proposed");
}

#[test]
fn accept_shorthand_equals_state_accepted() {
    let repo_a = support::repo::TestRepo::new();
    let paths_a = RepoPaths::from_repo_root(repo_a.path());
    init::init_forum(&paths_a).unwrap();
    setup_rfc(repo_a.path());
    // Move to under-review first
    let out = run(repo_a.path(), &["state", "RFC-0001", "proposed"]);
    assert!(out.status.success());
    let out = run(repo_a.path(), &["state", "RFC-0001", "under-review"]);
    assert!(out.status.success());
    // Add required summary
    let out = run(repo_a.path(), &["summary", "RFC-0001", "Looks good"]);
    assert!(out.status.success());
    let out = run(
        repo_a.path(),
        &["accept", "RFC-0001", "--approve", "human/alice"],
    );
    assert!(out.status.success());
    let state_a = replay(repo_a.path(), "RFC-0001");

    let repo_b = support::repo::TestRepo::new();
    let paths_b = RepoPaths::from_repo_root(repo_b.path());
    init::init_forum(&paths_b).unwrap();
    setup_rfc(repo_b.path());
    let out = run(repo_b.path(), &["state", "RFC-0001", "proposed"]);
    assert!(out.status.success());
    let out = run(repo_b.path(), &["state", "RFC-0001", "under-review"]);
    assert!(out.status.success());
    let out = run(repo_b.path(), &["summary", "RFC-0001", "Looks good"]);
    assert!(out.status.success());
    let out = run(
        repo_b.path(),
        &["state", "RFC-0001", "accepted", "--approve", "human/alice"],
    );
    assert!(out.status.success());
    let state_b = replay(repo_b.path(), "RFC-0001");

    assert_eq!(state_a.status, "accepted");
    assert_eq!(state_b.status, "accepted");
}
