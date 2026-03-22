mod support;

use std::process::Command;

#[test]
fn say_subcommand_shows_node_shorthand_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("say")
        .output()
        .expect("failed to run git-forum say");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("node shorthands"),
        "should suggest node shorthands; stderr: {stderr}"
    );
    assert!(
        stderr.contains("git forum claim"),
        "should mention claim shorthand; stderr: {stderr}"
    );
}

#[test]
fn revise_body_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("revise-body")
        .output()
        .expect("failed to run git-forum revise-body");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum revise"),
        "should suggest revise command; stderr: {stderr}"
    );
}

#[test]
fn create_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("create")
        .output()
        .expect("failed to run git-forum create");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum new"),
        "should suggest new command; stderr: {stderr}"
    );
}

#[test]
fn add_subcommand_shows_hint() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("add")
        .output()
        .expect("failed to run git-forum add");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("git forum node add"),
        "should suggest node add command; stderr: {stderr}"
    );
}

#[test]
fn unknown_subcommand_shows_help_llm_fallback() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .arg("nonexistent")
        .output()
        .expect("failed to run git-forum nonexistent");

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
    assert!(
        stderr.contains("--help-llm"),
        "should suggest --help-llm; stderr: {stderr}"
    );
}
