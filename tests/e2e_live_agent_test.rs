mod support;

use std::time::Duration;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::EventType;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;
use support::agent_adapter::{AgentAdapter, AgentRunResult};
use support::claude_adapter::{self, ClaudeCodeAdapter};
use support::repo::TestRepo;
use support::report;
use support::scenario;
use support::worktree;

fn parse_timeout_env() -> Duration {
    let secs: u64 = std::env::var("GIT_FORUM_AGENT_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    Duration::from_secs(secs)
}

fn parse_model_env() -> String {
    std::env::var("GIT_FORUM_AGENT_MODEL")
        .unwrap_or_else(|_| claude_adapter::DEFAULT_MODEL.to_string())
}

/// Build a map from expected thread refs to actual thread IDs by matching titles.
///
/// The scenario defines threads with expected sequential IDs (RFC-0001, ISSUE-0001, etc.)
/// but live agents create threads in unpredictable order. This function discovers actual
/// IDs by replaying all threads and matching their titles to the scenario definitions.
fn remap_expected_outcomes(
    git: &GitOps,
    expected: &[scenario::ExpectedOutcome],
    scenario: &scenario::ScenarioDef,
) -> Vec<scenario::ExpectedOutcome> {
    use std::collections::HashMap;

    // Build expected_ref -> title map from scenario thread definitions
    // The scenario assumes sequential IDs: first RFC created = RFC-0001, etc.
    let mut ref_to_title: HashMap<String, String> = HashMap::new();
    let mut rfc_counter = 0u32;
    let mut issue_counter = 0u32;
    for phase in &scenario.phases {
        for t in &phase.threads {
            let (prefix, counter) = match t.kind {
                git_forum::internal::event::ThreadKind::Rfc => ("RFC", &mut rfc_counter),
                git_forum::internal::event::ThreadKind::Issue => ("ISSUE", &mut issue_counter),
                git_forum::internal::event::ThreadKind::Dec => ("DEC", &mut rfc_counter),
                git_forum::internal::event::ThreadKind::Task => ("TASK", &mut issue_counter),
            };
            *counter += 1;
            let expected_ref = format!("{prefix}-{counter:04}");
            ref_to_title.insert(expected_ref, t.title.clone());
        }
    }

    // Build title -> actual_id map from live threads
    let thread_ids = thread::list_thread_ids(git).unwrap_or_default();
    let mut title_to_actual: HashMap<String, String> = HashMap::new();
    for id in &thread_ids {
        if let Ok(state) = thread::replay_thread(git, id) {
            title_to_actual
                .entry(state.title.clone())
                .or_insert_with(|| id.clone());
        }
    }

    // Remap each expected outcome
    expected
        .iter()
        .map(|exp| {
            let actual_ref = ref_to_title
                .get(&exp.thread_ref)
                .and_then(|title| title_to_actual.get(title))
                .cloned()
                .unwrap_or_else(|| exp.thread_ref.clone());

            scenario::ExpectedOutcome {
                thread_ref: actual_ref,
                expected_status: exp.expected_status.clone(),
                acceptable_statuses: exp.acceptable_statuses.clone(),
                min_nodes: exp.min_nodes,
                expected_evidence_count: exp.expected_evidence_count,
                expected_link_count: exp.expected_link_count,
            }
        })
        .collect()
}

#[test]
#[ignore]
fn e2e_live_agent_calculator_scenario() {
    if std::env::var("GIT_FORUM_LIVE_AGENT").unwrap_or_default() != "1" {
        println!("Skipping: set GIT_FORUM_LIVE_AGENT=1 to run");
        return;
    }

    let scenario = scenario::calculator_scenario();
    let expected = scenario::calculator_expected_outcomes();
    let timeout = parse_timeout_env();
    let model = parse_model_env();
    println!("Using model: {model}");

    // 1. Setup: repo + init forum + seed commit
    let repo = TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create initial commit for worktree support
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

    // 2. Create worktrees for all actors
    let base_dir = repo.path().join("worktrees");
    std::fs::create_dir_all(&base_dir).unwrap();
    let worktrees = worktree::setup_actor_worktrees(repo.path(), &scenario.actors, &base_dir);

    // 3. Check claude CLI is available; if not, mark INCOMPLETE
    if !claude_adapter::is_available() {
        println!("INCOMPLETE: claude CLI not found on PATH");
        return;
    }

    // Resolve git-forum binary path for prompts
    let git_forum_binary = env!("CARGO_BIN_EXE_git-forum");

    // 4. Execute phases in order, with participating actors running concurrently per phase
    let mut all_agent_results: Vec<AgentRunResult> = Vec::new();

    for (phase_index, phase) in scenario.phases.iter().enumerate() {
        // Determine which actors participate in this phase
        let mut phase_actors: Vec<&str> = Vec::new();
        for t in &phase.threads {
            if !phase_actors.contains(&t.creator.as_str()) {
                phase_actors.push(&t.creator);
            }
        }
        for n in &phase.nodes {
            if !phase_actors.contains(&n.actor.as_str()) {
                phase_actors.push(&n.actor);
            }
        }
        for t in &phase.transitions {
            if !phase_actors.contains(&t.actor.as_str()) {
                phase_actors.push(&t.actor);
            }
        }
        for e in &phase.evidence {
            if !phase_actors.contains(&e.actor.as_str()) {
                phase_actors.push(&e.actor);
            }
        }
        for l in &phase.links {
            if !phase_actors.contains(&l.actor.as_str()) {
                phase_actors.push(&l.actor);
            }
        }

        println!(
            "--- Phase '{}': running {} agent(s) concurrently ---",
            phase.name,
            phase_actors.len()
        );

        let phase_results = std::thread::scope(|scope| {
            let mut handles = Vec::new();

            for actor_name in &phase_actors {
                let actor_name = (*actor_name).to_string();
                let actor_def = scenario
                    .actors
                    .iter()
                    .find(|a| a.name == actor_name)
                    .unwrap();
                let (wt_path, _git) = worktrees.get(actor_name.as_str()).unwrap();
                let prompt = ClaudeCodeAdapter::build_prompt(
                    actor_def,
                    &scenario,
                    phase_index,
                    phase,
                    git_forum_binary,
                );
                let wt_path = wt_path.clone();
                let model = model.clone();

                handles.push(scope.spawn(move || {
                    let adapter =
                        ClaudeCodeAdapter::new(wt_path, timeout, actor_name.as_str(), &model);
                    let result = adapter.execute_task(&prompt);
                    (actor_name, model, result)
                }));
            }

            let mut results = Vec::new();
            for handle in handles {
                results.push(handle.join().unwrap());
            }
            results
        });

        for (actor_name, model, result) in phase_results {
            println!(
                "  {} exit={:?} duration={:.1}s success={}",
                actor_name,
                result.exit_code,
                result.duration.as_secs_f64(),
                result.success
            );
            if !result.success {
                let stderr_preview: String = result.stderr.chars().take(500).collect();
                println!("  {} stderr: {stderr_preview}", actor_name);
            }

            let command_args = vec![
                "claude".to_string(),
                "-p".to_string(),
                "<prompt>".to_string(),
                "--allowed-tools".to_string(),
                "Bash".to_string(),
                "--model".to_string(),
                model.clone(),
                "--max-budget-usd".to_string(),
                "0.50".to_string(),
            ];

            all_agent_results.push(AgentRunResult {
                actor_name,
                model,
                command_args,
                tasks: vec![result],
                completed: true,
                error: None,
            });
        }
    }

    // 5. Build report (all 6 RFC-0003 sections)
    let git = GitOps::new(repo.path().to_path_buf());

    // Remap expected outcomes using title-based discovery (agents create threads in
    // unpredictable order, so expected IDs like RFC-0001 may not match actual IDs)
    let remapped_expected = remap_expected_outcomes(&git, &expected, &scenario);
    println!("--- Thread ID remapping ---");
    for (orig, remapped) in expected.iter().zip(remapped_expected.iter()) {
        if orig.thread_ref != remapped.thread_ref {
            println!("  {} -> {}", orig.thread_ref, remapped.thread_ref);
        }
    }

    let mut scenario_report =
        report::build_report(&git, &remapped_expected, &all_agent_results, None);

    // Populate agent_results so AI analysis can see agent outputs
    scenario_report.agent_results = all_agent_results;

    // 5b. Generate AI usability analysis
    println!("--- Generating AI usability analysis ---");
    scenario_report.ai_usability_analysis =
        report::generate_ai_usability_analysis(&scenario_report, &model);

    let markdown = report::render_markdown(&scenario_report);
    println!("{markdown}");

    // 6. Write report to ./tmp/ (project dir, gitignored)
    let report_dir = std::path::Path::new("tmp");
    std::fs::create_dir_all(report_dir).unwrap();
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let report_path = report_dir.join(format!("e2e_live_agent_{timestamp}.md"));
    std::fs::write(&report_path, &markdown).unwrap();
    println!("Report written to: {}", report_path.display());

    // 7. Structural assertions only (non-deterministic agent behavior)
    let thread_ids = thread::list_thread_ids(&git).unwrap();
    assert!(
        !thread_ids.is_empty(),
        "agent should have created at least one thread"
    );

    // All forum refs should replay without error
    for id in &thread_ids {
        thread::replay_thread(&git, id).unwrap_or_else(|_| panic!("replay failed for {id}"));
    }

    // No duplicate event IDs
    let mut distinct_actors = std::collections::HashSet::new();
    let mut saw_state_change = false;
    let mut saw_collaborative_thread = false;
    let mut all_event_ids: Vec<String> = Vec::new();
    for id in &thread_ids {
        let state = thread::replay_thread(&git, id).unwrap();
        let mut thread_actors = std::collections::HashSet::new();
        for ev in &state.events {
            assert!(
                !all_event_ids.contains(&ev.event_id),
                "duplicate event_id: {}",
                ev.event_id
            );
            all_event_ids.push(ev.event_id.clone());
            distinct_actors.insert(ev.actor.clone());
            thread_actors.insert(ev.actor.clone());
            if ev.event_type == EventType::State {
                saw_state_change = true;
            }
        }
        if thread_actors.len() >= 2 {
            saw_collaborative_thread = true;
        }
    }

    assert!(
        distinct_actors.len() >= 2,
        "live run should record events from multiple actors"
    );
    assert!(
        saw_collaborative_thread,
        "live run should include at least one collaboratively updated thread"
    );
    assert!(
        saw_state_change,
        "live run should discover and execute at least one state transition"
    );
}
