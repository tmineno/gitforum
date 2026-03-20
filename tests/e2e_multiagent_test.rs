mod support;

use std::collections::HashMap;
use std::path::Path;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::StepClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::NodeType;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::say;
use git_forum::internal::thread;
use git_forum::internal::verify;
use support::repo::TestRepo;
use support::report;
use support::scenario::{self, ScenarioDef};

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

struct Agent {
    name: String,
    git: GitOps,
    clock: StepClock,
}

impl Agent {
    fn new(name: &str, git: GitOps) -> Self {
        let base = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        Self {
            name: name.to_string(),
            git,
            clock: StepClock::new(base, chrono::Duration::seconds(10)),
        }
    }
}

fn setup_scenario(scenario: &ScenarioDef) -> (TestRepo, Vec<Agent>, Policy) {
    let repo = TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create a real commit so evidence can reference it
    std::fs::write(repo.path().join("README.md"), "# Calculator\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(repo.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(repo.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();

    // Create agents from scenario actor definitions
    let agents: Vec<Agent> = scenario
        .actors
        .iter()
        .map(|a| Agent::new(&a.name, GitOps::new(repo.path().to_path_buf())))
        .collect();

    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap();

    (repo, agents, policy)
}

fn agent_by_name<'a>(agents: &'a [Agent], name: &str) -> &'a Agent {
    agents.iter().find(|a| a.name == name).unwrap()
}

fn empty_policy() -> Policy {
    Policy {
        roles: HashMap::new(),
        guards: vec![],
    }
}

// ---------------------------------------------------------------------------
// Phase 1: RFC Review
// ---------------------------------------------------------------------------

struct RfcIds {
    rfc_0001: String, // Calculator engine (accepted)
    rfc_0002: String, // Input validation (rejected)
    rfc_0003: String, // CLI interface (draft, left open)
}

fn phase_rfc_review(agents: &[Agent], scenario: &ScenarioDef) -> RfcIds {
    let phase = &scenario.phases[0]; // rfc-review

    // --- RFC-0001: Calculator engine ---
    let t0 = &phase.threads[0];
    let alice = agent_by_name(agents, &t0.creator);
    let rfc_0001 = create::create_thread(
        &alice.git,
        t0.kind,
        &t0.title,
        Some(&t0.body),
        &alice.name,
        &alice.clock,
    )
    .unwrap();
    assert_eq!(rfc_0001, "RFC-0001");

    // Nodes for RFC-0001
    let rfc1_nodes: Vec<&_> = phase
        .nodes
        .iter()
        .filter(|n| n.thread_ref == "RFC-0001")
        .collect();

    // Track node IDs for nodes that need resolving
    let mut resolve_ids: Vec<String> = Vec::new();

    for node_def in &rfc1_nodes {
        let agent = agent_by_name(agents, &node_def.actor);
        let node_id = say::say_node(
            &agent.git,
            &rfc_0001,
            node_def.node_type,
            &node_def.body,
            &agent.name,
            &agent.clock,
            None,
        )
        .unwrap();

        if node_def.should_resolve {
            resolve_ids.push(node_id);
        }
    }

    // Resolve nodes that need resolving (alice resolves them)
    for node_id in &resolve_ids {
        say::resolve_node(&alice.git, &rfc_0001, node_id, &alice.name, &alice.clock).unwrap();
    }

    // State transitions for RFC-0001
    for trans in phase
        .transitions
        .iter()
        .filter(|t| t.thread_ref == "RFC-0001")
    {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &rfc_0001,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    let state = thread::replay_thread(&alice.git, &rfc_0001).unwrap();
    assert_eq!(state.status, "accepted");
    assert!(state.open_objections().is_empty());
    assert!(state.open_actions().is_empty());
    assert!(state.latest_summary().is_some());

    // --- RFC-0002: Input validation (rejected) ---
    let t1 = &phase.threads[1];
    let bob = agent_by_name(agents, &t1.creator);
    let rfc_0002 = create::create_thread(
        &bob.git,
        t1.kind,
        &t1.title,
        Some(&t1.body),
        &bob.name,
        &bob.clock,
    )
    .unwrap();
    assert_eq!(rfc_0002, "RFC-0002");

    // Nodes for RFC-0002
    for node_def in phase.nodes.iter().filter(|n| n.thread_ref == "RFC-0002") {
        let agent = agent_by_name(agents, &node_def.actor);
        say::say_node(
            &agent.git,
            &rfc_0002,
            node_def.node_type,
            &node_def.body,
            &agent.name,
            &agent.clock,
            None,
        )
        .unwrap();
    }

    // State transitions for RFC-0002
    for trans in phase
        .transitions
        .iter()
        .filter(|t| t.thread_ref == "RFC-0002")
    {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &rfc_0002,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    let state = thread::replay_thread(&alice.git, &rfc_0002).unwrap();
    assert_eq!(state.status, "rejected");

    // --- RFC-0003: CLI interface (left in draft) ---
    let t2 = &phase.threads[2];
    let copilot = agent_by_name(agents, &t2.creator);
    let rfc_0003 = create::create_thread(
        &copilot.git,
        t2.kind,
        &t2.title,
        Some(&t2.body),
        &copilot.name,
        &copilot.clock,
    )
    .unwrap();
    assert_eq!(rfc_0003, "RFC-0003");

    let state = thread::replay_thread(&alice.git, &rfc_0003).unwrap();
    assert_eq!(state.status, "draft");

    RfcIds {
        rfc_0001,
        rfc_0002,
        rfc_0003,
    }
}

// ---------------------------------------------------------------------------
// Phase 2: Implementation (issues linked to RFCs)
// ---------------------------------------------------------------------------

struct IssueIds {
    issue_0001: String, // Add/sub (closed)
    issue_0002: String, // Mul/div (closed)
    issue_0003: String, // Div by zero (closed, with commit evidence)
    issue_0004: String, // Contention test
}

fn phase_implementation(
    agents: &[Agent],
    rfcs: &RfcIds,
    repo_path: &Path,
    scenario: &ScenarioDef,
) -> IssueIds {
    let phase = &scenario.phases[1]; // implementation

    // --- ISSUE-0001: Add/sub ---
    let t0 = &phase.threads[0];
    let alice = agent_by_name(agents, &t0.creator);
    let issue_0001 = create::create_thread(
        &alice.git,
        t0.kind,
        &t0.title,
        Some(&t0.body),
        &alice.name,
        &alice.clock,
    )
    .unwrap();
    assert_eq!(issue_0001, "ISSUE-0001");

    // Links for ISSUE-0001
    for link in phase
        .links
        .iter()
        .filter(|l| l.from_thread_ref == "ISSUE-0001")
    {
        let agent = agent_by_name(agents, &link.actor);
        evidence_ops::add_thread_link(
            &agent.git,
            &issue_0001,
            &rfcs.rfc_0001,
            &link.rel,
            &agent.name,
            &agent.clock,
        )
        .unwrap();
    }

    // State transition for ISSUE-0001
    for trans in phase
        .transitions
        .iter()
        .filter(|t| t.thread_ref == "ISSUE-0001")
    {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &issue_0001,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    // --- ISSUE-0002: Mul/div ---
    let t1 = &phase.threads[1];
    let bob = agent_by_name(agents, &t1.creator);
    let issue_0002 = create::create_thread(
        &bob.git,
        t1.kind,
        &t1.title,
        Some(&t1.body),
        &bob.name,
        &bob.clock,
    )
    .unwrap();
    assert_eq!(issue_0002, "ISSUE-0002");

    for link in phase
        .links
        .iter()
        .filter(|l| l.from_thread_ref == "ISSUE-0002")
    {
        let agent = agent_by_name(agents, &link.actor);
        evidence_ops::add_thread_link(
            &agent.git,
            &issue_0002,
            &rfcs.rfc_0001,
            &link.rel,
            &agent.name,
            &agent.clock,
        )
        .unwrap();
    }

    for trans in phase
        .transitions
        .iter()
        .filter(|t| t.thread_ref == "ISSUE-0002")
    {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &issue_0002,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    // --- ISSUE-0003: Div by zero (with commit evidence) ---
    let t2 = &phase.threads[2];
    let carol = agent_by_name(agents, &t2.creator);
    let issue_0003 = create::create_thread(
        &carol.git,
        t2.kind,
        &t2.title,
        Some(&t2.body),
        &carol.name,
        &carol.clock,
    )
    .unwrap();
    assert_eq!(issue_0003, "ISSUE-0003");

    // Create a commit to use as evidence
    std::fs::write(repo_path.join("div_guard.rs"), "fn div(a: f64, b: f64) -> Result<f64, &'static str> { if b == 0.0 { Err(\"div by zero\") } else { Ok(a / b) } }\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "div_guard.rs"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "add div-by-zero guard"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();

    // Add evidence from scenario
    for ev in phase
        .evidence
        .iter()
        .filter(|e| e.thread_ref == "ISSUE-0003")
    {
        let agent = agent_by_name(agents, &ev.actor);
        evidence_ops::add_evidence(
            &agent.git,
            &issue_0003,
            ev.kind.clone(),
            "HEAD",
            None,
            &agent.name,
            &agent.clock,
        )
        .unwrap();
    }

    for trans in phase
        .transitions
        .iter()
        .filter(|t| t.thread_ref == "ISSUE-0003")
    {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &issue_0003,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    // --- ISSUE-0004: Contention test issue ---
    let t3 = &phase.threads[3];
    let issue_creator = agent_by_name(agents, &t3.creator);
    let issue_0004 = create::create_thread(
        &issue_creator.git,
        t3.kind,
        &t3.title,
        Some(&t3.body),
        &issue_creator.name,
        &issue_creator.clock,
    )
    .unwrap();
    assert_eq!(issue_0004, "ISSUE-0004");

    // Verify closed issues
    for id in [&issue_0001, &issue_0002, &issue_0003] {
        let state = thread::replay_thread(&alice.git, id).unwrap();
        assert_eq!(state.status, "closed", "{id} should be closed");
    }
    let state = thread::replay_thread(&alice.git, &issue_0004).unwrap();
    assert_eq!(state.status, "open");

    // Verify evidence on ISSUE-0003
    let state = thread::replay_thread(&alice.git, &issue_0003).unwrap();
    assert_eq!(state.evidence_items.len(), 1);

    // Verify links
    let state = thread::replay_thread(&alice.git, &issue_0001).unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].rel, "implements");

    IssueIds {
        issue_0001,
        issue_0002,
        issue_0003,
        issue_0004,
    }
}

// ---------------------------------------------------------------------------
// Phase 3: Verify
// ---------------------------------------------------------------------------

fn phase_verify(agents: &[Agent], rfcs: &RfcIds, issues: &IssueIds, policy: &Policy) {
    let git = &agents[0].git;
    let all_ids = [
        &rfcs.rfc_0001,
        &rfcs.rfc_0002,
        &rfcs.rfc_0003,
        &issues.issue_0001,
        &issues.issue_0002,
        &issues.issue_0003,
        &issues.issue_0004,
    ];

    for thread_id in &all_ids {
        let report = verify::verify_thread(git, thread_id, policy).unwrap();
        assert!(
            report.passed(),
            "verify failed for {}: {:?}",
            thread_id,
            report.violations
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 4: Contention (concurrent writes)
// ---------------------------------------------------------------------------

fn phase_contention(agents: &[Agent], issue_id: &str) -> report::ContentionReport {
    let alice = &agents[0];
    let bob = &agents[1];

    let mut success_count = 0usize;
    let mut retry_count = 0usize;
    let mut conflict_errors: Vec<String> = Vec::new();

    std::thread::scope(|s| {
        let h1 = s.spawn(|| {
            let mut retries = 0;
            let mut errors = Vec::new();
            loop {
                let result = say::say_node(
                    &alice.git,
                    issue_id,
                    NodeType::Claim,
                    "Alice's concurrent note",
                    &alice.name,
                    &alice.clock,
                    None,
                );
                match result {
                    Ok(_) => return (true, retries, errors),
                    Err(e) => {
                        errors.push(format!("alice: {e}"));
                        retries += 1;
                        if retries > 5 {
                            return (false, retries, errors);
                        }
                    }
                }
            }
        });

        let h2 = s.spawn(|| {
            let mut retries = 0;
            let mut errors = Vec::new();
            loop {
                let result = say::say_node(
                    &bob.git,
                    issue_id,
                    NodeType::Claim,
                    "Bob's concurrent note",
                    &bob.name,
                    &bob.clock,
                    None,
                );
                match result {
                    Ok(_) => return (true, retries, errors),
                    Err(e) => {
                        errors.push(format!("bob: {e}"));
                        retries += 1;
                        if retries > 5 {
                            return (false, retries, errors);
                        }
                    }
                }
            }
        });

        let (ok1, r1, e1) = h1.join().unwrap();
        let (ok2, r2, e2) = h2.join().unwrap();
        if ok1 {
            success_count += 1;
        }
        if ok2 {
            success_count += 1;
        }
        retry_count = r1 + r2;
        conflict_errors.extend(e1);
        conflict_errors.extend(e2);
    });

    // Both should eventually succeed (git-forum uses CAS on refs)
    assert_eq!(success_count, 2, "both concurrent writes should succeed");

    // Verify final state has exactly 2 nodes
    let state = thread::replay_thread(&alice.git, "ISSUE-0004").unwrap();
    assert_eq!(
        state.nodes.len(),
        2,
        "contention test should have exactly 2 nodes"
    );

    report::ContentionReport {
        success_count,
        retry_count,
        conflict_errors,
    }
}

// ---------------------------------------------------------------------------
// Phase: Expanded lifecycle (missing transitions)
// ---------------------------------------------------------------------------

fn phase_expanded_lifecycle(agents: &[Agent], scenario: &ScenarioDef) {
    let phase = &scenario.phases[2]; // expanded-lifecycle

    // Create RFC-0004 and ISSUE-0005
    let t0 = &phase.threads[0]; // RFC-0004
    let bob = agent_by_name(agents, &t0.creator);
    let rfc_0004 = create::create_thread(
        &bob.git,
        t0.kind,
        &t0.title,
        Some(&t0.body),
        &bob.name,
        &bob.clock,
    )
    .unwrap();
    assert_eq!(rfc_0004, "RFC-0004");

    let t1 = &phase.threads[1]; // ISSUE-0005
    let carol = agent_by_name(agents, &t1.creator);
    let issue_0005 = create::create_thread(
        &carol.git,
        t1.kind,
        &t1.title,
        Some(&t1.body),
        &carol.name,
        &carol.clock,
    )
    .unwrap();
    assert_eq!(issue_0005, "ISSUE-0005");

    // Execute all transitions in order
    for trans in &phase.transitions {
        let agent = agent_by_name(agents, &trans.actor);
        say::change_state(
            &agent.git,
            &trans.thread_ref,
            &trans.new_state,
            &trans.sign_actors,
            &agent.name,
            &agent.clock,
            &empty_policy(),
            say::StateChangeOptions::default(),
        )
        .unwrap();
    }

    // Verify final states
    let alice = agent_by_name(agents, "human/alice");
    let state = thread::replay_thread(&alice.git, "RFC-0003").unwrap();
    assert_eq!(
        state.status, "draft",
        "RFC-0003 should be reverted to draft"
    );

    let state = thread::replay_thread(&alice.git, &rfc_0004).unwrap();
    assert_eq!(state.status, "deprecated");

    let state = thread::replay_thread(&alice.git, &issue_0005).unwrap();
    assert_eq!(state.status, "open", "ISSUE-0005 should be reopened");

    let state = thread::replay_thread(&alice.git, "ISSUE-0001").unwrap();
    assert_eq!(state.status, "closed", "ISSUE-0001 should be re-closed");
}

// ---------------------------------------------------------------------------
// Phase 6: CLI smoke tests
// ---------------------------------------------------------------------------

fn cli_smoke_tests(repo_path: &Path) {
    let binary = env!("CARGO_BIN_EXE_git-forum");

    // 1. list all threads
    let output = std::process::Command::new(binary)
        .args(["ls"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "ls failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RFC-0001"));
    assert!(stdout.contains("ISSUE-0001"));

    // 2. show a specific thread
    let output = std::process::Command::new(binary)
        .args(["show", "RFC-0001"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "show RFC-0001 failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Calculator engine"));

    // 3. verify a thread
    let output = std::process::Command::new(binary)
        .args(["verify", "ISSUE-0001"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "verify ISSUE-0001 failed");

    // 4. rfc ls
    let output = std::process::Command::new(binary)
        .args(["rfc", "ls"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "rfc ls failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RFC-0001"));
    assert!(!stdout.contains("ISSUE"), "rfc ls should not show issues");

    // 5. issue ls
    let output = std::process::Command::new(binary)
        .args(["issue", "ls"])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "issue ls failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ISSUE-0001"));
    assert!(!stdout.contains("RFC"), "issue ls should not show RFCs");
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

#[test]
fn e2e_multiagent_calculator_scenario() {
    let scenario = scenario::calculator_scenario();
    let expected = scenario::calculator_expected_outcomes();
    let (repo, agents, policy) = setup_scenario(&scenario);

    // Phase 1: RFC review
    let rfcs = phase_rfc_review(&agents, &scenario);

    // Phase 2: Implementation (issues linked to RFCs)
    let issues = phase_implementation(&agents, &rfcs, repo.path(), &scenario);

    // Phase 3: Verify all threads against policy
    phase_verify(&agents, &rfcs, &issues, &policy);

    // Phase 3b: Expanded lifecycle (missing transitions + node types)
    phase_expanded_lifecycle(&agents, &scenario);

    // Phase 4: Contention (concurrent writes to ISSUE-0004)
    let contention = phase_contention(&agents, &issues.issue_0004);

    // Phase 5: Report (using shared report module)
    let scenario_report = report::build_report(&agents[0].git, &expected, &[], Some(contention));
    let markdown = report::render_markdown(&scenario_report);
    println!("{markdown}");

    // Phase 6: CLI smoke tests
    cli_smoke_tests(repo.path());

    // Cross-cutting assertions
    let all_ids = thread::list_thread_ids(&agents[0].git).unwrap();
    assert_eq!(all_ids.len(), 9, "should have 9 threads total");

    // No duplicate event IDs across threads
    let mut all_event_ids: Vec<String> = Vec::new();
    for id in &all_ids {
        let state = thread::replay_thread(&agents[0].git, id).unwrap();
        for ev in &state.events {
            assert!(
                !all_event_ids.contains(&ev.event_id),
                "duplicate event_id: {}",
                ev.event_id
            );
            all_event_ids.push(ev.event_id.clone());
        }
    }
}
