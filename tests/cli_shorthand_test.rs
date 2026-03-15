mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::NodeType;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

#[test]
fn question_command_creates_question_node() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let create_rfc = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "Parser rewrite"])
        .output()
        .expect("failed to create rfc");
    assert!(create_rfc.status.success());

    let ask = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["question", "RFC-0001", "What compatibility risks remain?"])
        .output()
        .expect("failed to add question");
    assert!(ask.status.success());

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].node_type, NodeType::Question);
    assert_eq!(state.nodes[0].body, "What compatibility risks remain?");
}
