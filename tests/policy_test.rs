//! Policy parser/lint integration tests.
//!
//! The bulk of policy-shape coverage now lives inside
//! `src/internal/policy.rs` (`policy_load_tests`, `legacy_detection_tests`,
//! `evaluate_tests`, `category_registry_tests`) — moved there with the
//! SPEC-3.0 §3.2/§3.3 rewrite (task `9635buy0`, pre-flight P5). This
//! file covers integration-level wiring: the parser reads
//! `.forum/policy.toml` from a real init, and registry overrides
//! actually shape live workflows (initial status, valid transitions).

mod support;

use git_forum::internal::policy::Policy;
use support::forum::setup;

#[test]
fn policy_loads_from_initialized_repo() {
    let (_repo, _git, paths) = setup();
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();
    assert!(policy.category("rfc").is_some());
    assert!(policy.category("task").is_some());
}

#[test]
fn registry_override_changes_initial_status_for_new_threads() {
    // SPEC-3.0 §3.1: a built-in override in `.forum/policy.toml`
    // affects the new-thread initial status. The default rfc starts at
    // `draft`; override pins it to `open`.
    let (repo, _git, paths) = setup();
    let policy_path = paths.dot_forum.join("policy.toml");
    std::fs::write(
        &policy_path,
        r#"
[categories.rfc]
initial_status = "open"
statuses = ["open", "review", "done", "rejected", "deprecated"]
transitions = [
  "open->review",
  "open->rejected",
  "review->done",
  "review->rejected",
  "done->deprecated",
  "rejected->deprecated",
]
"#,
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "rfc", "Override test", "--as", "human/alice"])
        .output()
        .expect("git-forum new should succeed");
    assert!(
        output.status.success(),
        "git-forum new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let policy = Policy::load(&policy_path).unwrap();
    let reg = policy.effective_registry();
    let rfc = reg
        .get("rfc")
        .expect("rfc still in registry after override");
    assert_eq!(rfc.initial_status, "open");
    // Override removed `draft->open`; the only edge from open is now to
    // review/rejected (no `open->done` either).
    assert!(rfc.allows_transition("open", "review"));
    assert!(!rfc.allows_transition("draft", "open"));
    assert!(!rfc.allows_transition("open", "done"));
}

#[test]
fn registry_override_constrains_valid_transitions() {
    // SPEC-3.0 §3.1: a transition omitted from the override registry is
    // not valid. `task` is overridden to drop `open->working` and
    // `working->done`, so a state command targeting those edges errors.
    let (repo, _git, paths) = setup();
    let policy_path = paths.dot_forum.join("policy.toml");
    std::fs::write(
        &policy_path,
        r#"
[categories.task]
initial_status = "open"
statuses = ["open", "done", "rejected", "deprecated"]
transitions = [
  "open->done",
  "open->rejected",
  "done->open",
  "rejected->open",
  "done->deprecated",
  "rejected->deprecated",
]
"#,
    )
    .unwrap();

    // Create a task thread; should land in `open`.
    let create = std::process::Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "task", "Constrained task", "--as", "human/alice"])
        .output()
        .expect("git-forum new should succeed");
    assert!(create.status.success());
    let stdout = String::from_utf8_lossy(&create.stdout);
    let thread_id = stdout
        .lines()
        .find_map(|l| l.strip_prefix("Created "))
        .expect("created line")
        .trim()
        .to_string();

    // `pend` is the task verb that targets `working` (per SPEC-2.0 §9.3).
    // The override drops `working` from `statuses`, so the resolution
    // either fails because `working` is unknown or the registry rejects
    // the transition.
    let pend = std::process::Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["pend", &thread_id, "--as", "human/alice"])
        .output()
        .expect("git-forum pend should run");
    assert!(
        !pend.status.success(),
        "pend should fail under the override (working dropped from statuses): stdout={}",
        String::from_utf8_lossy(&pend.stdout)
    );

    // `close` targets `done`. The override keeps `open->done`, so this
    // succeeds even though we removed the working/review intermediaries.
    let close = std::process::Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["close", &thread_id, "--as", "human/alice"])
        .output()
        .expect("git-forum close should run");
    assert!(
        close.status.success(),
        "close should succeed with open->done in override: {}",
        String::from_utf8_lossy(&close.stderr)
    );
}
