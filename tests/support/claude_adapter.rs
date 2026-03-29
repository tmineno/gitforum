#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use git_forum::internal::event::{NodeType, ThreadKind};

use super::agent_adapter::{AgentAdapter, AgentTaskResult};
use super::scenario::{ActorDef, PhaseDef, ScenarioDef};

/// Default model for live-agent mode.
pub const DEFAULT_MODEL: &str = "sonnet";

/// Adapter that executes tasks via the Claude Code CLI (`claude`).
pub struct ClaudeCodeAdapter {
    worktree_path: PathBuf,
    timeout: Duration,
    actor_name: String,
    model: String,
}

impl ClaudeCodeAdapter {
    pub fn new(worktree_path: PathBuf, timeout: Duration, actor_name: &str, model: &str) -> Self {
        Self {
            worktree_path,
            timeout,
            actor_name: actor_name.to_string(),
            model: model.to_string(),
        }
    }

    /// Build a map from scenario labels (e.g. "RFC-0001", "ASK-0001") to
    /// thread metadata. Labels use the same prefixes as `ThreadKind::id_prefix()`
    /// so they match the `thread_ref` values in scenario definitions.
    fn thread_catalog(scenario: &ScenarioDef) -> HashMap<String, (ThreadKind, String, String)> {
        let mut refs = HashMap::new();
        let mut rfc_counter = 0u32;
        let mut issue_counter = 0u32;

        for phase in &scenario.phases {
            for thread in &phase.threads {
                let (prefix, counter) = match thread.kind {
                    ThreadKind::Rfc => {
                        rfc_counter += 1;
                        ("RFC", rfc_counter)
                    }
                    ThreadKind::Issue => {
                        issue_counter += 1;
                        ("ASK", issue_counter)
                    }
                    ThreadKind::Dec => {
                        rfc_counter += 1;
                        ("DEC", rfc_counter)
                    }
                    ThreadKind::Task => {
                        issue_counter += 1;
                        ("JOB", issue_counter)
                    }
                };
                refs.insert(
                    format!("{prefix}-{counter:04}"),
                    (thread.kind, thread.title.clone(), thread.body.clone()),
                );
            }
        }

        refs
    }

    fn known_threads_for_phase(
        scenario: &ScenarioDef,
        phase_index: usize,
    ) -> Vec<(ThreadKind, String)> {
        let mut known = Vec::new();
        for phase in scenario.phases.iter().take(phase_index + 1) {
            for thread in &phase.threads {
                known.push((thread.kind, thread.title.clone()));
            }
        }
        known
    }

    fn describe_node_goal(node_type: NodeType) -> &'static str {
        match node_type {
            NodeType::Claim => "add a concrete claim",
            NodeType::Question => "ask a clarifying question",
            NodeType::Objection => "raise an objection",
            NodeType::Evidence => "record supporting evidence",
            NodeType::Summary => "write a summary",
            NodeType::Action => "record an action item",
            NodeType::Risk => "surface a risk",
            NodeType::Review => "add an overall review",
            NodeType::Alternative => "record a considered alternative",
            NodeType::Assumption => "record an assumption",
        }
    }

    fn phase_brief(phase_name: &str) -> &'static str {
        match phase_name {
            "rfc-review" => {
                "Drive early design discussion. Open the needed RFCs, review each other's proposals, and use the forum state to decide whether anything should advance, stay open, or be rejected. Weak or duplicate proposals should be rejected outright — do not leave everything in draft."
            }
            "implementation" => {
                "Turn the active design into implementation work. Open issues, connect implementation back to design, attach evidence from the repo when it helps, and close work only when the repo state supports it."
            }
            "expanded-lifecycle" => {
                "Revisit existing work and exercise non-happy-path lifecycle decisions. This includes rejection (draft->rejected or under-review->rejected), deprecation (accepted->deprecated or rejected->deprecated), reverting to draft (proposed->draft), and reopening closed/rejected items (closed->open, rejected->open). Decide for yourself when rollback, rejection, reopening, or retirement is warranted based on the thread content."
            }
            "contention" => {
                "This phase intentionally creates concurrent activity on the same thread. Coordinate through the shared forum state, refresh before writing, and retry after CAS conflicts."
            }
            _ => "Inspect the current forum state and decide what to do next based on the project goals below.",
        }
    }

    /// Build a prompt for an agent given its role, the scenario, and current phase.
    pub fn build_prompt(
        actor: &ActorDef,
        scenario: &ScenarioDef,
        phase_index: usize,
        phase: &PhaseDef,
        git_forum_binary: &str,
    ) -> String {
        let mut prompt = String::new();
        let thread_catalog = Self::thread_catalog(scenario);
        let known_threads = Self::known_threads_for_phase(scenario, phase_index);

        let collaborators: Vec<&str> = scenario
            .actors
            .iter()
            .filter(|other| {
                other.name != actor.name
                    && (phase.threads.iter().any(|t| t.creator == other.name)
                        || phase.nodes.iter().any(|n| n.actor == other.name)
                        || phase.transitions.iter().any(|t| t.actor == other.name)
                        || phase.evidence.iter().any(|e| e.actor == other.name)
                        || phase.links.iter().any(|l| l.actor == other.name))
            })
            .map(|other| other.name.as_str())
            .collect();

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

        // Tool context
        prompt.push_str(&format!(
            "## Setup\n\n\
             The git-forum binary is at: {}\n\
             Your actor identity is already set via GIT_FORUM_ACTOR env var.\n\
             The forum is already initialized in this repo.\n\
             Run `{} --help-llm` to see full documentation.\n\
             Other actors in this phase are running concurrently in separate worktrees.\n\n",
            git_forum_binary, git_forum_binary
        ));

        // Phase instructions
        prompt.push_str(&format!("## Current Phase: {}\n\n", phase.name));
        prompt.push_str(Self::phase_brief(&phase.name));
        prompt.push_str("\n\n");

        if !collaborators.is_empty() {
            prompt.push_str("### Concurrent collaborators\n");
            for name in &collaborators {
                prompt.push_str(&format!("- {name}\n"));
            }
            prompt.push('\n');
        }

        if !known_threads.is_empty() {
            prompt.push_str("### Known project threads by title\n");
            prompt.push_str("Discover actual thread IDs yourself with `ls` and `show`.\n");
            for (kind, title) in &known_threads {
                prompt.push_str(&format!("- {} \"{}\"\n", kind, title));
            }
            prompt.push('\n');
        }

        prompt.push_str("### Discovery rules\n");
        prompt.push_str(
            "- Do not assume thread IDs, command sequence, or valid state transitions from this prompt.\n",
        );
        prompt.push_str(
            "- Inspect the repo and current forum state before acting. Use `ls`, `show`, `show <ID> --what-next`, `verify`, and `--help-llm` as needed.\n",
        );
        prompt.push_str(
            "- If another actor has not created or updated something yet, refresh and adapt instead of guessing IDs.\n",
        );
        prompt.push_str(
            "- Decide the concrete git-forum procedure yourself from the live repo state.\n\n",
        );

        // Project goals for this actor
        prompt.push_str("## Your goals\n");

        let actor_threads: Vec<&_> = phase
            .threads
            .iter()
            .filter(|t| t.creator == actor.name)
            .collect();
        for thread in &actor_threads {
            prompt.push_str(&format!(
                "- Introduce a {} titled \"{}\" about: {}\n",
                thread.kind, thread.title, thread.body
            ));
        }

        let actor_nodes: Vec<&_> = phase
            .nodes
            .iter()
            .filter(|n| n.actor == actor.name)
            .collect();
        for node in &actor_nodes {
            let thread_title = thread_catalog
                .get(&node.thread_ref)
                .map(|(_, title, _)| title.as_str())
                .unwrap_or(node.thread_ref.as_str());
            prompt.push_str(&format!(
                "- In \"{}\", {} around: {}\n",
                thread_title,
                Self::describe_node_goal(node.node_type),
                node.body
            ));
            if node.should_resolve {
                prompt.push_str(
                    "  If the discussion is satisfactorily addressed during this phase, close the loop using the appropriate forum workflow.\n",
                );
            }
        }

        let mut lifecycle_threads: Vec<String> = phase
            .transitions
            .iter()
            .filter(|t| t.actor == actor.name)
            .filter_map(|t| {
                thread_catalog
                    .get(&t.thread_ref)
                    .map(|(_, title, _)| title.clone())
            })
            .collect();
        lifecycle_threads.sort();
        lifecycle_threads.dedup();
        for title in &lifecycle_threads {
            if phase.name == "expanded-lifecycle" {
                prompt.push_str(&format!(
                    "- Reassess the lifecycle of \"{}\". Consider whether rejection, deprecation, reverting to draft, or reopening is appropriate based on the thread content. Use `show <ID> --what-next` to see valid transitions and guard requirements.\n",
                    title
                ));
            } else {
                prompt.push_str(&format!(
                    "- Reassess the lifecycle of \"{}\" from the live thread state. Choose any next actions or state changes only after checking what the tool says is valid.\n",
                    title
                ));
            }
        }

        let actor_evidence: Vec<&_> = phase
            .evidence
            .iter()
            .filter(|e| e.actor == actor.name)
            .collect();
        for evidence in &actor_evidence {
            let thread_title = thread_catalog
                .get(&evidence.thread_ref)
                .map(|(_, title, _)| title.as_str())
                .unwrap_or(evidence.thread_ref.as_str());
            prompt.push_str(&format!(
                "- Attach repository-backed evidence to \"{}\". Choose the evidence form and reference from the current repo state.\n",
                thread_title
            ));
        }

        let actor_links: Vec<&_> = phase
            .links
            .iter()
            .filter(|l| l.actor == actor.name)
            .collect();
        for link in &actor_links {
            let from_title = thread_catalog
                .get(&link.from_thread_ref)
                .map(|(_, title, _)| title.as_str())
                .unwrap_or(link.from_thread_ref.as_str());
            let to_title = thread_catalog
                .get(&link.to_thread_ref)
                .map(|(_, title, _)| title.as_str())
                .unwrap_or(link.to_thread_ref.as_str());
            prompt.push_str(&format!(
                "- Connect \"{}\" back to \"{}\" in whatever way best reflects the project relationship.\n",
                from_title, to_title
            ));
        }

        // Usage hints
        if actor_threads.is_empty()
            && actor_nodes.is_empty()
            && lifecycle_threads.is_empty()
            && actor_evidence.is_empty()
            && actor_links.is_empty()
        {
            prompt.push_str("- No specific ownership was assigned in this phase. Inspect the repo and help other actors by advancing the project where it is obviously useful.\n");
        }

        prompt.push_str("\n## Execution constraints:\n");
        prompt.push_str(&format!(
            "Use the git-forum binary at `{}` to execute all commands.\n",
            git_forum_binary
        ));
        prompt.push_str("Execute your work via Bash.\n");
        prompt.push_str("Do NOT create files or run other programs unless git-forum itself requires Git state that already exists in the repo.\n");
        prompt.push_str("Before any state change, re-read the relevant thread so you do not race on stale assumptions.\n");

        prompt.push_str(&format!(
            "\n## Before finishing\n\
             1. For each thread you modified, run `{bin} status <ID>` to check for unresolved items.\n\
             2. For each thread you modified, run `{bin} show <ID> --what-next` to confirm no blocked transitions remain.\n\
             3. If open actions block a close, resolve each with `{bin} resolve <THREAD> <NODE>` or use --resolve-open-actions.\n",
            bin = git_forum_binary
        ));

        prompt
    }
}

impl AgentAdapter for ClaudeCodeAdapter {
    fn execute_task(&self, prompt: &str) -> AgentTaskResult {
        let start = Instant::now();

        let spawn_result = Command::new("claude")
            .args([
                "-p",
                prompt,
                "--allowed-tools",
                "Bash",
                "--model",
                &self.model,
                "--max-budget-usd",
                "0.50",
            ])
            .current_dir(&self.worktree_path)
            .env("GIT_FORUM_ACTOR", &self.actor_name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match spawn_result {
            Ok(child) => child,
            Err(e) => {
                return AgentTaskResult {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("Failed to spawn claude: {e}"),
                    duration: start.elapsed(),
                    exit_code: None,
                };
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stdout) = stdout {
                let _ = stdout.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_end(&mut buf);
            }
            buf
        });

        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if start.elapsed() >= self.timeout {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    let stdout = String::from_utf8_lossy(&stdout_handle.join().unwrap_or_default())
                        .to_string();
                    let mut stderr =
                        String::from_utf8_lossy(&stderr_handle.join().unwrap_or_default())
                            .to_string();
                    if !stderr.is_empty() {
                        stderr.push('\n');
                    }
                    stderr.push_str(&format!("Failed while waiting for claude: {e}"));
                    return AgentTaskResult {
                        success: false,
                        stdout,
                        stderr,
                        duration: start.elapsed(),
                        exit_code: None,
                    };
                }
            }
        };

        let stdout = String::from_utf8_lossy(&stdout_handle.join().unwrap_or_default()).to_string();
        let mut stderr =
            String::from_utf8_lossy(&stderr_handle.join().unwrap_or_default()).to_string();
        if timed_out {
            if !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "Agent process timed out after {:.1}s",
                self.timeout.as_secs_f64()
            ));
        }

        let duration = start.elapsed();

        match status {
            Some(status) => AgentTaskResult {
                success: status.success(),
                stdout,
                stderr,
                duration,
                exit_code: status.code(),
            },
            None => AgentTaskResult {
                success: false,
                stdout,
                stderr,
                duration,
                exit_code: None,
            },
        }
    }

    fn shutdown(&mut self) {}

    fn platform_name(&self) -> &str {
        "claude-code"
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

#[cfg(test)]
mod tests {
    use super::ClaudeCodeAdapter;
    use crate::support::scenario;

    #[test]
    fn live_prompt_requires_discovery_instead_of_hardcoded_ids() {
        let scenario = scenario::calculator_scenario();
        let actor = &scenario.actors[0];
        let phase = &scenario.phases[0];

        let prompt = ClaudeCodeAdapter::build_prompt(actor, &scenario, 0, phase, "git-forum");

        assert!(prompt.contains("Do not assume thread IDs"));
        assert!(prompt.contains("show <ID> --what-next"));
        assert!(prompt.contains("Other actors in this phase are running concurrently"));
        assert!(!prompt.contains("RFC-0001"));
        assert!(!prompt.contains("ASK-0001"));
        assert!(!prompt.contains("### State transitions"));
        assert!(!prompt.contains("Change RFC-0001"));
    }
}
