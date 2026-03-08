mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::SequentialIdGenerator;
use git_forum::internal::run_ops;
use git_forum::internal::show;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    (repo, git)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn make_thread(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
        &SequentialIdGenerator::new("t"),
    )
    .unwrap()
}

// ---- Evidence tests ----

#[test]
fn add_evidence_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Benchmark,
        "bench/result.csv",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.evidence_items.len(), 1);
    assert_eq!(state.evidence_items[0].kind, EvidenceKind::Benchmark);
    assert_eq!(state.evidence_items[0].ref_target, "bench/result.csv");
    assert!(!state.evidence_items[0].evidence_id.is_empty());
}

#[test]
fn evidence_id_is_populated_from_commit_sha() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let commit_sha = evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Commit,
        "abc123",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.evidence_items[0].evidence_id, commit_sha);
}

#[test]
fn show_includes_evidence_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Benchmark,
        "bench/result.csv",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);
    assert!(out.contains("evidence: 1"));
    assert!(out.contains("benchmark"));
    assert!(out.contains("bench/result.csv"));
}

// ---- Thread link tests ----

#[test]
fn add_thread_link_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_thread_link(
        &git,
        &thread_id,
        "ISSUE-0001",
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, "ISSUE-0001");
    assert_eq!(state.links[0].rel, "implements");
}

#[test]
fn show_includes_links_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_thread_link(
        &git,
        &thread_id,
        "ISSUE-0001",
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);
    assert!(out.contains("links: 1"));
    assert!(out.contains("ISSUE-0001"));
    assert!(out.contains("implements"));
}

// ---- Run tests ----

#[test]
fn spawn_run_creates_ref_and_spawn_event() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let run_label = run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();
    assert_eq!(run_label, "RUN-0001");

    // ref exists
    let run = run_ops::read_run(&git, &run_label).unwrap();
    assert_eq!(run.run_label, "RUN-0001");
    assert_eq!(run.thread_id, thread_id);
    assert_eq!(run.actor_id, "ai/reviewer");

    // spawn event in thread
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.run_labels.contains(&"RUN-0001".to_string()));
}

#[test]
fn spawn_run_increments_label() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let label1 = run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();
    let label2 = run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();
    assert_eq!(label1, "RUN-0001");
    assert_eq!(label2, "RUN-0002");
}

#[test]
fn list_runs_returns_all_in_order() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();
    run_ops::spawn_run(&git, &thread_id, "ai/summarizer", &fixed_clock()).unwrap();

    let runs = run_ops::list_runs(&git).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_label, "RUN-0001");
    assert_eq!(runs[1].run_label, "RUN-0002");
}

#[test]
fn show_includes_runs_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state);
    assert!(out.contains("runs: 1"));
    assert!(out.contains("RUN-0001"));
}

#[test]
fn render_run_ls_empty() {
    assert_eq!(show::render_run_ls(&[]), "no runs found\n");
}

#[test]
fn render_run_show_contains_key_fields() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let run_label = run_ops::spawn_run(&git, &thread_id, "ai/reviewer", &fixed_clock()).unwrap();
    let run = run_ops::read_run(&git, &run_label).unwrap();

    let out = show::render_run_show(&run);
    assert!(out.contains("RUN-0001"));
    assert!(out.contains("running"));
    assert!(out.contains("ai/reviewer"));
    assert!(out.contains(&thread_id));
}
