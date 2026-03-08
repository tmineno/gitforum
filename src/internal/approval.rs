#![allow(dead_code)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Approval mechanism (spec §7.7).
/// MVP requires only `recorded`; cryptographic variants are future work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMechanism {
    Recorded,
}

/// An approval record attached to a state or decision event (spec §7.7).
/// Full implementation in M3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub actor_id: String,
    pub approved_at: DateTime<Utc>,
    pub mechanism: ApprovalMechanism,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_ref: Option<String>,
}
