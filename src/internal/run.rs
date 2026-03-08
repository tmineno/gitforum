use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of an AI run (spec §7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failure,
    Partial,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Partial => "partial",
        };
        f.write_str(s)
    }
}

/// AI model information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub name: String,
}

/// Prompt provenance hashes and context references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInfo {
    pub system_hash: String,
    pub task_hash: String,
    #[serde(default)]
    pub context_refs: Vec<String>,
}

/// The result / outcome of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// An AI run record (spec §7.6).
///
/// Stored as `run.json` inside the Git commit at `refs/forum/runs/<run_label>`.
/// `run_id` (commit SHA) and `run_label` (ref suffix) are not serialised; they are
/// populated from the Git ref after loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    /// Git commit SHA — populated from Git, not stored in JSON.
    #[serde(skip)]
    pub run_id: String,
    /// Human-readable label (e.g. `RUN-0001`) — populated from ref name, not stored in JSON.
    #[serde(skip)]
    pub run_label: String,
    pub actor_id: String,
    pub thread_id: String,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RunResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<serde_json::Value>,
}
