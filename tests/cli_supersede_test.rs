//! `git forum supersede <OLD> --by <NEW>` — ticket `eda1c050`.
//!
//! Coverage:
//! - End-to-end: `<old>` lands in `deprecated`, gets a `superseded-by`
//!   link to `<new>`, gets a comment node (default body when omitted).
//! - `<new>` gets the symmetric `supersedes` link so `show <new>`
//!   surfaces the relationship without a reverse-link index.
//! - Custom comment body via `--body` overrides the default.
//! - `ls --status rejected` does NOT surface superseded threads (the
//!   key acceptance criterion that motivates this verb over `reject`).
//! - `<old>` and `--by` referring to the same thread is rejected.

#[allow(dead_code)]
mod support;

use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::NodeKind;
use git_forum::internal::thread;

use support::cli::{fresh_repo, make_thread_via_cli, run, run_ok};

fn replay(repo_path: &std::path::Path, thread_id: &str) -> thread::ThreadState {
    let git = GitOps::new(repo_path.to_path_buf());
    thread::replay_thread(&git, thread_id).unwrap()
}

#[test]
fn supersede_default_body_lands_old_in_deprecated_with_link_and_comment() {
    let repo = fresh_repo();
    let old = make_thread_via_cli(repo.path(), "issue", "Old issue", "stub");
    let new = make_thread_via_cli(repo.path(), "issue", "New issue", "stub");

    let out = run_ok(
        repo.path(),
        &["supersede", &old, "--by", &new, "--as", "human/alice"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("{old} -> deprecated")),
        "expected `<old> -> deprecated` line, got: {stdout}"
    );

    let old_state = replay(repo.path(), &old);
    assert_eq!(old_state.status, "deprecated");
    assert!(
        old_state
            .links
            .iter()
            .any(|l| l.target_thread_id == new && l.rel == "superseded-by"),
        "old.links missing superseded-by edge to new: {:?}",
        old_state.links
    );

    let comment = old_state
        .nodes
        .iter()
        .find(|n| matches!(n.record.kind, NodeKind::Comment))
        .expect("supersede should attach a comment node on <old>");
    assert!(
        comment.body.contains(&new),
        "default comment body should mention <new>, got: {}",
        comment.body
    );
}

#[test]
fn supersede_custom_body_overrides_default() {
    let repo = fresh_repo();
    let old = make_thread_via_cli(repo.path(), "issue", "Old issue", "stub");
    let new = make_thread_via_cli(repo.path(), "issue", "New issue", "stub");

    run_ok(
        repo.path(),
        &[
            "supersede",
            &old,
            "--by",
            &new,
            "--body",
            "Folded into the new ticket; see acceptance plan there.",
            "--as",
            "human/alice",
        ],
    );

    let old_state = replay(repo.path(), &old);
    let comment = old_state
        .nodes
        .iter()
        .find(|n| matches!(n.record.kind, NodeKind::Comment))
        .expect("comment node must exist");
    assert_eq!(
        comment.body,
        "Folded into the new ticket; see acceptance plan there."
    );
}

#[test]
fn supersede_writes_back_supersedes_link_on_new_side() {
    // Acceptance criterion: `show <new>` lists the supersede
    // relationship. With no reverse-link index in v3.0.0, this is
    // backed by writing the symmetric `supersedes` edge onto <new>.
    let repo = fresh_repo();
    let old = make_thread_via_cli(repo.path(), "issue", "Old issue", "stub");
    let new = make_thread_via_cli(repo.path(), "issue", "New issue", "stub");

    run_ok(
        repo.path(),
        &["supersede", &old, "--by", &new, "--as", "human/alice"],
    );

    let new_state = replay(repo.path(), &new);
    assert!(
        new_state
            .links
            .iter()
            .any(|l| l.target_thread_id == old && l.rel == "supersedes"),
        "new.links should contain `supersedes` edge to old: {:?}",
        new_state.links
    );

    let show_out = run_ok(repo.path(), &["show", &new]);
    let body = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        body.contains("supersedes"),
        "show <new> should surface the `supersedes` relationship, got:\n{body}"
    );
    assert!(
        body.contains(&old),
        "show <new> should reference the old thread id, got:\n{body}"
    );
}

#[test]
fn supersede_does_not_surface_in_ls_status_rejected() {
    // Acceptance criterion: superseded threads do not pollute
    // `ls --status rejected`. They are `deprecated`, not `rejected`.
    let repo = fresh_repo();
    let old = make_thread_via_cli(repo.path(), "issue", "Old issue", "stub");
    let new = make_thread_via_cli(repo.path(), "issue", "New issue", "stub");
    let plain_reject = make_thread_via_cli(repo.path(), "issue", "Bad bug", "stub");

    run_ok(
        repo.path(),
        &["supersede", &old, "--by", &new, "--as", "human/alice"],
    );
    run_ok(
        repo.path(),
        &["reject", &plain_reject, "--as", "human/alice"],
    );

    let ls_rejected = run_ok(repo.path(), &["ls", "--status", "rejected"]);
    let body = String::from_utf8_lossy(&ls_rejected.stdout);
    assert!(
        body.contains(&plain_reject),
        "rejected thread should appear in `ls --status rejected`: {body}"
    );
    assert!(
        !body.contains(&old),
        "superseded thread must NOT appear in `ls --status rejected`: {body}"
    );

    let ls_deprecated = run_ok(repo.path(), &["ls", "--status", "deprecated"]);
    let body = String::from_utf8_lossy(&ls_deprecated.stdout);
    assert!(
        body.contains(&old),
        "superseded thread should appear in `ls --status deprecated`: {body}"
    );
}

#[test]
fn supersede_rejects_self_reference() {
    let repo = fresh_repo();
    let id = make_thread_via_cli(repo.path(), "issue", "Self", "stub");
    let out = run(
        repo.path(),
        &["supersede", &id, "--by", &id, "--as", "human/alice"],
    );
    assert!(
        !out.status.success(),
        "supersede with same <old> and --by should fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("different threads"),
        "expected hint about different threads, got: {stderr}"
    );
}

#[test]
fn supersede_works_on_rfc_in_open_state() {
    // Sanity-check the new direct `open->deprecated` edge in the rfc
    // category: supersede must succeed without `--fast-track`.
    let repo = fresh_repo();
    let old = run_ok(
        repo.path(),
        &[
            "new",
            "rfc",
            "Old RFC",
            "--body",
            "## Goal\nA.\n## Non-goals\nB.\n## Context\nC.\n## Proposal\nD.",
        ],
    );
    let old_id = support::cli::extract_created_id(&old);
    run_ok(repo.path(), &["propose", &old_id, "--as", "human/alice"]);
    let new = run_ok(
        repo.path(),
        &[
            "new",
            "rfc",
            "New RFC",
            "--body",
            "## Goal\nA.\n## Non-goals\nB.\n## Context\nC.\n## Proposal\nD.",
        ],
    );
    let new_id = support::cli::extract_created_id(&new);

    run_ok(
        repo.path(),
        &["supersede", &old_id, "--by", &new_id, "--as", "human/alice"],
    );
    let s = replay(repo.path(), &old_id);
    assert_eq!(s.status, "deprecated");
}
