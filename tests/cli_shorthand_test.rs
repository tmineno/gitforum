mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::NodeType;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

fn extract_created_id(output: &Output) -> String {
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

#[test]
fn question_command_creates_question_node() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let create_rfc = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "rfc",
            "new",
            "Parser rewrite",
            "--body",
            "## Goal\nRewrite the parser.",
        ])
        .output()
        .expect("failed to create rfc");
    assert!(create_rfc.status.success());
    let rfc_id = extract_created_id(&create_rfc);

    let ask = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["question", &rfc_id, "What compatibility risks remain?"])
        .output()
        .expect("failed to add question");
    assert!(ask.status.success());

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &rfc_id).unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_type, NodeType::Question);
    assert_eq!(state.nodes[0].body, "What compatibility risks remain?");
}
