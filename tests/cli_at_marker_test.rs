//! `@<id>` is accepted on input and never printed (SPEC-3.0 §6,
//! ADR-014, thread `cxpv15ny`).

mod support;

use serde_json::Value;
use support::cli::{extract_created_id, fresh_repo, run, run_ok};

#[test]
fn show_accepts_at_marker_input() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "Marker input"]));

    let plain = run_ok(repo.path(), &["show", &id]);
    let marked = run_ok(repo.path(), &["show", &format!("@{id}")]);
    assert_eq!(
        plain.stdout, marked.stdout,
        "`show @<id>` should resolve to the same thread as `show <id>`"
    );
}

#[test]
fn show_header_prints_bare_id() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "Bare header"]));

    let out = run_ok(repo.path(), &["show", &format!("@{id}"), "--full"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&id), "{stdout}");
    assert!(!stdout.contains(&format!("@{id}")), "{stdout}");
}

#[test]
fn supersede_default_comment_uses_bare_id() {
    let repo = fresh_repo();
    let old = extract_created_id(&run_ok(repo.path(), &["new", "task", "Old"]));
    let new = extract_created_id(&run_ok(repo.path(), &["new", "task", "New"]));
    run_ok(
        repo.path(),
        &["supersede", &old, "--by", &format!("@{new}")],
    );

    let out = run_ok(repo.path(), &["show", &old, "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let bodies: Vec<&str> = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["body"].as_str())
        .collect();
    assert!(
        bodies
            .iter()
            .any(|b| b.trim_end() == format!("Superseded by {new}")),
        "bodies: {bodies:?}"
    );
}

#[test]
fn show_node_redirect_hint_uses_bare_id() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "Parent"]));
    run_ok(repo.path(), &["comment", &id, "a node"]);
    let out = run_ok(repo.path(), &["show", &id, "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let node_id = v["nodes"][0]["id"].as_str().unwrap().to_string();

    let out = run(repo.path(), &["show", &node_id]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("it lives in thread {id}")),
        "{stderr}"
    );
    assert!(!stderr.contains(&format!("@{id}")), "{stderr}");
}
