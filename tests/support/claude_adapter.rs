#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use super::agent_adapter::{AgentAdapter, AgentTaskResult};
use super::scenario::{ActorDef, PhaseDef, ScenarioDef};

/// Adapter that executes tasks via the Claude Code CLI (`claude`).
pub struct ClaudeCodeAdapter {
    worktree_path: PathBuf,
    timeout: Duration,
    child: Option<Child>,
}

impl ClaudeCodeAdapter {
    pub fn new(worktree_path: PathBuf, timeout: Duration) -> Self {
        Self {
            worktree_path,
            timeout,
            child: None,
        }
    }

    /// Build a prompt for an agent given its role, the scenario, and current phase.
    pub fn build_prompt(actor: &ActorDef, scenario: &ScenarioDef, phase: &PhaseDef) -> String {
        let mut prompt = String::new();

        // Role context
        prompt.push_str(&format!(
            "You are {}, a {}. {}\n\n",
            actor.name, actor.role, actor.description
        ));

        // Project context
        prompt.push_str(&format!(
            "Project: {} — {}\n\n",
            scenario.name, scenario.description
        ));

        // Phase instructions
        prompt.push_str(&format!("## Current Phase: {}\n\n", phase.name));

        // Thread creation tasks for this actor
        let actor_threads: Vec<&_> = phase
            .threads
            .iter()
            .filter(|t| t.creator == actor.name)
            .collect();
        if !actor_threads.is_empty() {
            prompt.push_str("### Threads to create:\n");
            for t in &actor_threads {
                prompt.push_str(&format!(
                    "- Create {} titled \"{}\": {}\n",
                    t.kind, t.title, t.body
                ));
            }
            prompt.push('\n');
        }

        // Node tasks for this actor
        let actor_nodes: Vec<&_> = phase
            .nodes
            .iter()
            .filter(|n| n.actor == actor.name)
            .collect();
        if !actor_nodes.is_empty() {
            prompt.push_str("### Nodes to add:\n");
            for n in &actor_nodes {
                prompt.push_str(&format!(
                    "- Add {} to {}: \"{}\"\n",
                    n.node_type, n.thread_ref, n.body
                ));
                if n.should_resolve {
                    prompt.push_str("  (This node should be resolved after creation)\n");
                }
            }
            prompt.push('\n');
        }

        // State transitions for this actor
        let actor_transitions: Vec<&_> = phase
            .transitions
            .iter()
            .filter(|t| t.actor == actor.name)
            .collect();
        if !actor_transitions.is_empty() {
            prompt.push_str("### State transitions:\n");
            for t in &actor_transitions {
                prompt.push_str(&format!(
                    "- Change {} to state '{}'\n",
                    t.thread_ref, t.new_state
                ));
                if !t.sign_actors.is_empty() {
                    prompt.push_str(&format!("  (Signers: {})\n", t.sign_actors.join(", ")));
                }
            }
            prompt.push('\n');
        }

        // Evidence tasks for this actor
        let actor_evidence: Vec<&_> = phase
            .evidence
            .iter()
            .filter(|e| e.actor == actor.name)
            .collect();
        if !actor_evidence.is_empty() {
            prompt.push_str("### Evidence to attach:\n");
            for e in &actor_evidence {
                prompt.push_str(&format!(
                    "- Attach {} evidence to {}\n",
                    e.kind, e.thread_ref
                ));
            }
            prompt.push('\n');
        }

        // Link tasks for this actor
        let actor_links: Vec<&_> = phase
            .links
            .iter()
            .filter(|l| l.actor == actor.name)
            .collect();
        if !actor_links.is_empty() {
            prompt.push_str("### Links to create:\n");
            for l in &actor_links {
                prompt.push_str(&format!(
                    "- Link {} to {} (rel: {})\n",
                    l.from_thread_ref, l.to_thread_ref, l.rel
                ));
            }
            prompt.push('\n');
        }

        // Usage hints
        prompt.push_str("## Available commands:\n");
        prompt.push_str("Use `git forum` (or `git-forum`) CLI commands.\n");
        prompt.push_str("Run `git forum --help-llm` for full documentation.\n");

        prompt
    }
}

impl AgentAdapter for ClaudeCodeAdapter {
    fn execute_task(&self, prompt: &str) -> AgentTaskResult {
        let start = Instant::now();

        let result = Command::new("claude")
            .args([
                "-p",
                prompt,
                "--cwd",
                self.worktree_path.to_str().unwrap_or("."),
                "--allowedTools",
                "Bash",
                "--max-budget-usd",
                "0.50",
            ])
            .current_dir(&self.worktree_path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output();

        let duration = start.elapsed();

        match result {
            Ok(output) => {
                let success = output.status.success();
                AgentTaskResult {
                    success,
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    duration,
                    exit_code: output.status.code(),
                }
            }
            Err(e) => AgentTaskResult {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to spawn claude: {e}"),
                duration,
                exit_code: None,
            },
        }
    }

    fn shutdown(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }

    fn platform_name(&self) -> &str {
        "claude-code"
    }
}

impl Drop for ClaudeCodeAdapter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Check if the `claude` CLI is available on PATH.
pub fn is_available() -> bool {
    Command::new("claude")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
