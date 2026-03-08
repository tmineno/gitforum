#![allow(dead_code)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of an AI run (spec §7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Success,
    Failure,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInfo {
    pub system_hash: String,
    pub task_hash: String,
    #[serde(default)]
    pub context_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// An AI run record (spec §7.6).
/// Full implementation in M4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub actor_id: String,
    pub thread_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub model: ModelInfo,
    pub prompt: PromptInfo,
    pub result: RunResult,
    #[serde(default)]
    pub tool_calls: Vec<serde_json::Value>,
}
