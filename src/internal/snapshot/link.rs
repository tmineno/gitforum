//! `links.toml` model + serde.
//!
//! SPEC-3.0 §2.3 schema:
//!
//! ```toml
//! [[links]]
//! target = "abc123xy"
//! rel = "implements"
//! created_at = "2026-05-03T00:00:00Z"
//! created_by = "human/alice"
//! ```
//!
//! `from` is implicit (the enclosing snapshot's thread id) and is NOT
//! a struct field. `target`/`rel`/`created_at`/`created_by` are the
//! only valid keys; v2 names (`to`, `relation`, `actor`) MUST NOT
//! appear in the wire form.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::internal::error::ForumError;

/// One outgoing link from the enclosing snapshot's thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub target: String,
    pub rel: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// `links.toml` document.
///
/// Top-level shape is a single `[[links]]` array; the Rust field is
/// named `entries` to avoid shadowing the type name, with serde rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Links {
    #[serde(default, rename = "links", skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Link>,
}

impl Links {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize to a TOML string suitable for writing to `links.toml`.
    pub fn to_toml(&self) -> Result<String, ForumError> {
        toml::to_string(self)
            .map_err(|e| ForumError::SnapshotInvalid(format!("serialize links.toml: {e}")))
    }

    /// Parse a `links.toml` document.
    pub fn from_toml(s: &str) -> Result<Self, ForumError> {
        Ok(toml::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_link() -> Link {
        Link {
            target: "abc123xy".into(),
            rel: "implements".into(),
            created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            created_by: "human/alice".into(),
        }
    }

    #[test]
    fn round_trip_single_link() {
        let original = Links {
            entries: vec![sample_link()],
        };
        let s = original.to_toml().unwrap();
        let parsed = Links::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_empty() {
        let original = Links::default();
        let s = original.to_toml().unwrap();
        let parsed = Links::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
        assert!(original.is_empty());
    }

    #[test]
    fn key_fidelity_uses_spec_keys_only() {
        let s = Links {
            entries: vec![sample_link()],
        }
        .to_toml()
        .unwrap();
        // SPEC-3.0 §2.3 keys.
        assert!(s.contains("target = "), "missing `target`: {s}");
        assert!(s.contains("rel = "), "missing `rel`: {s}");
        assert!(s.contains("created_at = "), "missing `created_at`: {s}");
        assert!(s.contains("created_by = "), "missing `created_by`: {s}");
        // v2 / non-spec keys MUST NOT appear.
        assert!(!s.contains("from = "), "unexpected `from` key: {s}");
        assert!(!s.contains("to = "), "unexpected `to` key: {s}");
        assert!(!s.contains("relation = "), "unexpected `relation` key: {s}");
        assert!(!s.contains("actor = "), "unexpected `actor` key: {s}");
    }

    #[test]
    fn rejects_legacy_field_names() {
        // A v2-shaped document with `to`/`relation`/`actor` must not
        // deserialize into Link successfully.
        let bad = r#"
            [[links]]
            to = "abc123xy"
            relation = "implements"
            created_at = "2026-05-03T00:00:00Z"
            actor = "human/alice"
        "#;
        let err = Links::from_toml(bad).unwrap_err();
        match err {
            ForumError::Toml(_) => {}
            other => panic!("expected Toml deserialization error, got {other}"),
        }
    }

    #[test]
    fn parses_multiple_links_array() {
        let s = r#"
            [[links]]
            target = "thread-a"
            rel = "implements"
            created_at = "2026-05-03T00:00:00Z"
            created_by = "human/alice"

            [[links]]
            target = "thread-b"
            rel = "blocks"
            created_at = "2026-05-03T01:00:00Z"
            created_by = "ai/codex"
        "#;
        let parsed = Links::from_toml(s).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].target, "thread-a");
        assert_eq!(parsed.entries[1].rel, "blocks");
    }
}
