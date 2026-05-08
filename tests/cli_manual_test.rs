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

/// Ticket `wm25ip8y`: `git forum --help` is transformed by git into
/// `git help git-forum`, which falls back to `man git-forum`. We ship
/// a static man page at `doc/man/git-forum.1` so that lookup
/// succeeds. The binary's own `--help` (with hyphen) keeps printing
/// the standard clap top-level help.
#[test]
fn help_short_form_prints_top_level_synopsis() {
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .arg("--help")
        .output()
        .expect("failed to run git-forum --help");

    assert!(
        output.status.success(),
        "git-forum --help must exit successfully; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: git-forum"),
        "stdout should carry the clap usage line, got:\n{stdout}"
    );
    assert!(
        stdout.contains("--help-llm"),
        "stdout should mention --help-llm as the long-form entry point, got:\n{stdout}"
    );
    assert!(
        stdout.contains("These are common git-forum commands"),
        "stdout should include the GROUPED_HELP after-help block, got:\n{stdout}"
    );
}

#[test]
fn help_short_form_h_alias_matches_long_form() {
    let long = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .arg("--help")
        .output()
        .expect("failed to run git-forum --help");
    let short = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .arg("-h")
        .output()
        .expect("failed to run git-forum -h");

    assert!(long.status.success());
    assert!(short.status.success());
    assert_eq!(
        String::from_utf8_lossy(&long.stdout),
        String::from_utf8_lossy(&short.stdout),
        "`-h` and `--help` must produce identical output (clap convention)"
    );
}

/// Ship the man page in-tree so `git forum --help` (which becomes
/// `man git-forum` via git's help shim) has something to resolve once
/// the install instructions are followed. The file must exist, must
/// declare itself as section 1, and must reference the in-binary
/// `--help-llm` long-form entry point.
#[test]
fn manpage_is_present_and_well_formed() {
    let manpage = include_str!("../doc/man/git-forum.1");
    assert!(
        !manpage.trim().is_empty(),
        "doc/man/git-forum.1 must not be empty"
    );
    assert!(
        manpage.contains(".TH GIT-FORUM 1"),
        "manpage must declare section 1 via `.TH GIT-FORUM 1 ...`"
    );
    assert!(
        manpage.contains("--help-llm"),
        "manpage should point readers at --help-llm for the full reference"
    );
    assert!(
        manpage.contains("git-forum"),
        "manpage should name the binary"
    );
}
