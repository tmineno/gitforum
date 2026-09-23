//! `git forum show --json` (SPEC-3.0 §7, ticket `g0nh5fjf`).
//!
//! The payload exists so that an agent can read back what it wrote —
//! body, nodes, links, evidence — without parsing the human rendering.
//! These tests pin the key set and check each part round-trips from the
//! write that produced it.

mod support;

use serde_json::Value;
use support::cli::{extract_created_id, fresh_repo, run, run_ok};

fn show_json(repo: &support::repo::TestRepo, id: &str) -> Value {
    let out = run_ok(repo.path(), &["show", id, "--json"]);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "show --json is not valid JSON ({e}):\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_object()
        .expect("expected a JSON object")
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut k: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    k.sort();
    k
}

#[test]
fn show_json_round_trips_body_and_pins_top_level_keys() {
    let repo = fresh_repo();
    let body = "## Goal\n\nRead back what was written.\n";
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "task", "Round trip", "--body", body],
    ));

    let v = show_json(&repo, &id);
    assert_eq!(
        keys(&v),
        sorted(&[
            "id",
            "title",
            "category",
            "lifecycle",
            "tags",
            "status",
            "visibility",
            "branch",
            "created_at",
            "created_by",
            "updated_at",
            "body",
            "body_revision_count",
            "latest_summary",
            "nodes",
            "links",
            "evidence",
        ])
    );
    assert_eq!(v["id"], id.as_str());
    assert_eq!(v["title"], "Round trip");
    assert_eq!(v["body"].as_str().map(str::trim_end), Some(body.trim_end()));
    assert_eq!(v["visibility"], "private");
    assert!(v["branch"].is_null());
}

#[test]
fn show_json_body_is_null_not_missing_when_absent() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "No body"]));

    let v = show_json(&repo, &id);
    assert!(v.as_object().unwrap().contains_key("body"));
    assert!(v["body"].is_null(), "body: {}", v["body"]);
}

#[test]
fn show_json_lists_nodes_with_body_and_type() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "issue", "Discuss"]));
    run_ok(repo.path(), &["comment", &id, "First thought."]);

    let v = show_json(&repo, &id);
    let nodes = v["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    let node = &nodes[0];
    assert_eq!(
        keys(node),
        sorted(&[
            "id",
            "type",
            "status",
            "body",
            "created_at",
            "created_by",
            "updated_at",
            "updated_by",
            "reply_to",
        ])
    );
    assert_eq!(node["type"], "comment");
    assert_eq!(node["status"], "open");
    assert_eq!(
        node["body"].as_str().map(str::trim_end),
        Some("First thought.")
    );
}

#[test]
fn show_json_lists_evidence_rows() {
    let repo = fresh_repo();
    support::git::git(
        repo.path(),
        &[
            "commit",
            "--allow-empty",
            "--no-verify",
            "-m",
            "implementation",
        ],
    );
    let head = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(repo.path())
            // A git hook in a linked worktree exports absolute GIT_DIR /
            // GIT_INDEX_FILE; without this the command reads that worktree.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "With evidence"]));
    run_ok(
        repo.path(),
        &["evidence", "add", &id, "--kind", "commit", "--ref", "HEAD"],
    );

    let v = show_json(&repo, &id);
    let evidence = v["evidence"].as_array().expect("evidence array");
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        keys(&evidence[0]),
        sorted(&["id", "kind", "ref", "created_at", "created_by"])
    );
    assert_eq!(evidence[0]["kind"], "commit");
    assert_eq!(evidence[0]["ref"], head.as_str());
}

#[test]
fn show_json_lists_outgoing_links() {
    let repo = fresh_repo();
    let a = extract_created_id(&run_ok(repo.path(), &["new", "task", "From"]));
    let b = extract_created_id(&run_ok(repo.path(), &["new", "task", "To"]));
    run_ok(repo.path(), &["link", &a, &b, "--rel", "relates-to"]);

    let v = show_json(&repo, &a);
    let links = v["links"].as_array().expect("links array");
    assert_eq!(links.len(), 1);
    assert_eq!(keys(&links[0]), sorted(&["target", "rel"]));
    assert_eq!(links[0]["target"], b.as_str());
    assert_eq!(links[0]["rel"], "relates-to");
}

#[test]
fn show_json_rejects_other_view_flags() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "Flags"]));
    for flag in ["--what-next", "--tree", "--compact", "--with-timeline"] {
        let out = run(repo.path(), &["show", &id, "--json", flag]);
        assert!(!out.status.success(), "--json {flag} should be rejected");
    }
}

#[test]
fn show_default_tip_mentions_evidence() {
    let repo = fresh_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "task", "Tip"]));

    let out = run_ok(repo.path(), &["show", &id]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--full` for open items, conversations, links, evidence, and timeline"),
        "{stdout}"
    );
}
