use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::ForumError;

// --------------------------------------------------------------------
// v2 `NodeType` (the 12-variant enum carrying 1.x rhetorical labels)
// was relocated to `internal::legacy::event` by task `1v400j3l`.
// Only the migration consumer needs the legacy variants; runtime
// node code uses `NodeKind` (4 variants) below.
//
// The v2 `Node` struct below now stores `NodeKind` directly, with
// any 1.x rhetorical label preserved as a string in `legacy_subtype`.
// --------------------------------------------------------------------

/// A structured discussion node contributed via a `say` event.
///
/// Preconditions: constructed from a Say event during replay.
/// Postconditions: immutable after construction; state tracked via resolved/retracted flags.
/// Failure modes: none (plain data struct).
/// Side effects: none.
///
/// `Default` is derived so test sites can elide unset optional fields with
/// struct-update syntax (e.g. `Node { node_id: …, node_type: …, body: …, ..Default::default() }`),
/// matching the pattern used on `Event`.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub node_id: String,
    pub node_type: NodeKind,
    pub body: String,
    pub actor: String,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
    pub retracted: bool,
    pub incorporated: bool,
    pub reply_to: Option<String>,
    /// SPEC-2.0 §2.5: rhetorical-subtype label preserved when the canonical
    /// `node_type` is `Comment` but the user (or migration tool) recorded a
    /// 1.x label like `claim` / `summary` / `risk` / `review` / `question` /
    /// `evidence` / `alternative` / `assumption`. `None` for canonical types
    /// or for native 2.0 `comment` writes.
    pub legacy_subtype: Option<String>,
}

impl Node {
    /// True when the node is neither resolved, retracted, nor incorporated.
    pub fn is_open(&self) -> bool {
        !self.resolved && !self.retracted && !self.incorporated
    }

    /// Display label for the node's lifecycle state. Single source of truth
    /// for the `retracted | incorporated | resolved | open` cascade used by
    /// replay, the index, and rendering.
    pub fn status(&self) -> &'static str {
        if self.retracted {
            "retracted"
        } else if self.incorporated {
            "incorporated"
        } else if self.resolved {
            "resolved"
        } else {
            "open"
        }
    }
}

// --------------------------------------------------------------------
// SPEC-3.0 §2.2 `nodes/<id>.toml` shape.
//
// Strict 4-variant `NodeKind` (no claim/question/summary/risk/review/
// alternative/assumption) and a `NodeStatus` enum replace the v2
// `NodeType` carrying legacy variants and the three boolean status
// flags. The legacy `Node` struct above is retained for v2 compatibility;
// removal is tracked by task `913c4s9v`.
// --------------------------------------------------------------------

/// SPEC-3.0 §2.2 node type (strict canonical four).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    #[default]
    Comment,
    Approval,
    Objection,
    Action,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Comment => "comment",
            Self::Approval => "approval",
            Self::Objection => "objection",
            Self::Action => "action",
        })
    }
}

impl std::str::FromStr for NodeKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "comment" => Ok(Self::Comment),
            "approval" => Ok(Self::Approval),
            "objection" => Ok(Self::Objection),
            "action" => Ok(Self::Action),
            // Legacy 1.x rhetorical labels (claim/question/summary/risk/
            // review/alternative/assumption/evidence) are not valid for
            // 3.0 native writes — they collapse to `comment` and the
            // rhetorical label belongs in the migration archival
            // `legacy_label` field, not as a write-time type.
            other => Err(format!(
                "unknown node type '{other}'; SPEC-3.0 native types: comment, approval, objection, action"
            )),
        }
    }
}

/// SPEC-3.0 §2.2 node status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    #[default]
    Open,
    Resolved,
    Retracted,
    Incorporated,
}

/// SPEC-3.0 §2.2 node metadata (`nodes/<id>.toml`).
///
/// One file per node; the document is a flat key/value record (not a
/// table or array). The paired `nodes/<id>.md` body file is owned by
/// `snapshot::store`, not by this type.
///
/// `Default` is derived (task `1v400j3l`) so test
/// fixtures and migrate-projection helpers can elide unset optional
/// fields with struct-update syntax (`NodeRecord { id: …, kind: …,
/// ..Default::default() }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NodeRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub created_at: DateTime<Utc>,
    pub created_by: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Migration-only archival label (SPEC-3.0 §2.2). Ignored for live
    /// behavior; preserves the user's 1.x rhetorical label if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_label: Option<String>,
}

impl NodeRecord {
    pub fn to_toml(&self) -> Result<String, ForumError> {
        toml::to_string(self)
            .map_err(|e| ForumError::SnapshotInvalid(format!("serialize node toml: {e}")))
    }

    pub fn from_toml(s: &str) -> Result<Self, ForumError> {
        Ok(toml::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> NodeRecord {
        NodeRecord {
            id: "node1".into(),
            kind: NodeKind::Comment,
            status: NodeStatus::Open,
            created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            created_by: "human/alice".into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        }
    }

    #[test]
    fn node_record_round_trip_minimal() {
        let original = sample_record();
        let s = original.to_toml().unwrap();
        let parsed = NodeRecord::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn node_record_round_trip_with_optionals() {
        let original = NodeRecord {
            updated_at: Some("2026-05-03T01:00:00Z".parse().unwrap()),
            updated_by: Some("ai/codex".into()),
            reply_to: Some("parent_node".into()),
            legacy_label: Some("claim".into()),
            ..sample_record()
        };
        let s = original.to_toml().unwrap();
        let parsed = NodeRecord::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn node_record_uses_spec_keys() {
        let s = sample_record().to_toml().unwrap();
        assert!(s.contains("id = "), "missing `id`: {s}");
        assert!(s.contains("type = "), "missing `type`: {s}");
        assert!(s.contains("status = "), "missing `status`: {s}");
        assert!(s.contains("created_at = "), "missing `created_at`: {s}");
        assert!(s.contains("created_by = "), "missing `created_by`: {s}");
        // v2 field names MUST NOT appear.
        assert!(!s.contains("node_id = "), "unexpected `node_id`: {s}");
        assert!(!s.contains("node_type = "), "unexpected `node_type`: {s}");
        assert!(!s.contains("actor = "), "unexpected `actor`: {s}");
        assert!(
            !s.contains("resolved = "),
            "unexpected `resolved` flag: {s}"
        );
        assert!(
            !s.contains("retracted = "),
            "unexpected `retracted` flag: {s}"
        );
        assert!(
            !s.contains("incorporated = "),
            "unexpected `incorporated` flag: {s}"
        );
        assert!(
            !s.contains("legacy_subtype = "),
            "unexpected `legacy_subtype`: {s}"
        );
    }

    #[test]
    fn node_record_rejects_v2_field_names() {
        let bad = r#"
            node_id = "node1"
            node_type = "comment"
            actor = "human/alice"
            created_at = "2026-05-03T00:00:00Z"
            resolved = false
        "#;
        let err = NodeRecord::from_toml(bad).unwrap_err();
        assert!(matches!(err, ForumError::Toml(_)));
    }

    #[test]
    fn node_record_rejects_non_canonical_type() {
        for legacy_type in [
            "claim",
            "question",
            "summary",
            "risk",
            "review",
            "alternative",
            "assumption",
            "evidence",
        ] {
            let s = format!(
                r#"
                id = "node1"
                type = "{legacy_type}"
                status = "open"
                created_at = "2026-05-03T00:00:00Z"
                created_by = "human/alice"
                "#
            );
            let err = NodeRecord::from_toml(&s).unwrap_err();
            assert!(
                matches!(err, ForumError::Toml(_)),
                "expected Toml error for legacy type {legacy_type}, got {err}"
            );
        }
    }

    #[test]
    fn node_record_rejects_unknown_status() {
        let bad = r#"
            id = "node1"
            type = "comment"
            status = "deferred"
            created_at = "2026-05-03T00:00:00Z"
            created_by = "human/alice"
        "#;
        let err = NodeRecord::from_toml(bad).unwrap_err();
        assert!(matches!(err, ForumError::Toml(_)));
    }

    #[test]
    fn node_record_omits_unset_optionals() {
        let s = sample_record().to_toml().unwrap();
        assert!(
            !s.contains("updated_at"),
            "unset `updated_at` should be omitted: {s}"
        );
        assert!(
            !s.contains("updated_by"),
            "unset `updated_by` should be omitted: {s}"
        );
        assert!(
            !s.contains("reply_to"),
            "unset `reply_to` should be omitted: {s}"
        );
        assert!(
            !s.contains("legacy_label"),
            "unset `legacy_label` should be omitted: {s}"
        );
    }
}
