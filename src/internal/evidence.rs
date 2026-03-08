#![allow(dead_code)]
use serde::{Deserialize, Serialize};

/// Supported evidence kinds (spec §7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceKind {
    Commit,
    File,
    Hunk,
    Test,
    Benchmark,
    Doc,
    Thread,
    External,
}

/// Optional locator fields for an evidence reference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Locator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A reference to supporting evidence (spec §7.4).
/// Full implementation in M4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub ref_target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
}
