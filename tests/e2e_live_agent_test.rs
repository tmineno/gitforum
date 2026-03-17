mod support;

use std::time::Duration;

use git_forum::internal::config::RepoPaths;
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
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(repo.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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

    // 4. Execute phases sequentially
    let mut all_agent_results: Vec<AgentRunResult> = Vec::new();

    for phase in &scenario.phases {
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

        for actor_name in &phase_actors {
            let actor_def = scenario
                .actors
                .iter()
                .find(|a| a.name == *actor_name)
                .unwrap();

            let (wt_path, _git) = worktrees.get(*actor_name).unwrap();
            let prompt =
                ClaudeCodeAdapter::build_prompt(actor_def, &scenario, phase, git_forum_binary);

            println!(
                "--- Phase '{}': running agent for {} ---",
                phase.name, actor_name
            );

            let adapter = ClaudeCodeAdapter::new(wt_path.clone(), timeout, actor_name);
            let result = adapter.execute_task(&prompt);

            println!(
                "  exit={:?} duration={:.1}s success={}",
                result.exit_code,
                result.duration.as_secs_f64(),
                result.success
            );
            if !result.success {
                let stderr_preview: String = result.stderr.chars().take(500).collect();
                println!("  stderr: {stderr_preview}");
            }

            all_agent_results.push(AgentRunResult {
                actor_name: actor_name.to_string(),
                tasks: vec![result],
                completed: true,
                error: None,
            });
        }
    }

    // 5. Build and print report (all 6 RFC-0003 sections)
    let git = GitOps::new(repo.path().to_path_buf());
    let scenario_report = report::build_report(&git, &expected, &all_agent_results, None);
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
    let mut all_event_ids: Vec<String> = Vec::new();
    for id in &thread_ids {
        let state = thread::replay_thread(&git, id).unwrap();
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
