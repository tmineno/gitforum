#![allow(dead_code)]

use std::time::Duration;

/// Result of a single agent task execution.
pub struct AgentTaskResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub exit_code: Option<i32>,
}

/// Aggregated result of all tasks executed by an agent.
pub struct AgentRunResult {
    pub actor_name: String,
    pub model: String,
    pub command_args: Vec<String>,
    pub tasks: Vec<AgentTaskResult>,
    pub completed: bool,
    pub error: Option<String>,
}

/// Trait for adapters that execute scenario tasks via external agents.
pub trait AgentAdapter: Send {
    /// Execute a task by sending a prompt and capturing output.
    fn execute_task(&self, prompt: &str) -> AgentTaskResult;

    /// Gracefully shut down the adapter.
    fn shutdown(&mut self);

    /// Name of the platform (e.g., "claude-code").
    fn platform_name(&self) -> &str;
}
