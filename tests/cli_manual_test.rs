use std::process::Command;

#[test]
fn help_llm_prints_manual_verbatim() {
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .arg("--help-llm")
        .output()
        .expect("failed to run git-forum --help-llm");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    assert_eq!(stdout, include_str!("../doc/MANUAL.md"));
}
