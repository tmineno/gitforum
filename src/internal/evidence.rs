use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType};
use super::git_ops::GitOps;

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

impl std::fmt::Display for EvidenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Commit => "commit",
            Self::File => "file",
            Self::Hunk => "hunk",
            Self::Test => "test",
            Self::Benchmark => "benchmark",
            Self::Doc => "doc",
            Self::Thread => "thread",
            Self::External => "external",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for EvidenceKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "commit" => Ok(Self::Commit),
            "file" => Ok(Self::File),
            "hunk" => Ok(Self::Hunk),
            "test" => Ok(Self::Test),
            "benchmark" => Ok(Self::Benchmark),
            "doc" => Ok(Self::Doc),
            "thread" => Ok(Self::Thread),
            "external" => Ok(Self::External),
            _ => Err(format!(
                "unknown evidence kind '{s}'; valid: commit, file, hunk, test, benchmark, doc, thread, external"
            )),
        }
    }
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
///
/// `evidence_id` is not stored in JSON; it is populated from the enclosing event's `event_id`
/// (commit SHA) during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(skip)]
    pub evidence_id: String,
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub ref_target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
}

/// Add an evidence item to a thread via a Link event.
///
/// Preconditions: git is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Link event carrying the evidence is written to the thread.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn add_evidence(
    git: &GitOps,
    thread_id: &str,
    kind: EvidenceKind,
    ref_target: &str,
    locator: Option<Locator>,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let ref_target = canonicalize_evidence_ref(git, &kind, ref_target)?;
    let ev = Event::base(thread_id, EventType::Link, actor, clock).with_evidence(Evidence {
        evidence_id: String::new(),
        kind,
        ref_target,
        locator,
    });
    super::event::write_event(git, &ev)
}

fn canonicalize_evidence_ref(
    git: &GitOps,
    kind: &EvidenceKind,
    ref_target: &str,
) -> ForumResult<String> {
    match kind {
        EvidenceKind::Commit => git.resolve_commit(ref_target),
        _ => Ok(ref_target.to_string()),
    }
}

/// Add a link between two threads via a Link event.
///
/// Preconditions: git is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Link event with target and rel is written to the thread.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn add_thread_link(
    git: &GitOps,
    thread_id: &str,
    target_thread_id: &str,
    rel: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let ev = Event::base(thread_id, EventType::Link, actor, clock)
        .with_target_node_id(target_thread_id)
        .with_link_rel(rel);
    super::event::write_event(git, &ev)
}

// --------------------------------------------------------------------
// SPEC-3.0 §2.3 `evidence.toml` shape.
//
// The 3.0 record carries the snapshot-time metadata the SPEC requires
// (`id`, `kind`, `ref`, `created_at`, `created_by`). It lives alongside
// the legacy `Evidence` struct above; the legacy struct exists for the
// 2.x event-time write/read path until Phase 4 deletes it.
// --------------------------------------------------------------------

/// One `[[evidence]]` entry in `evidence.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub ref_target: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// `evidence.toml` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvidenceFile {
    #[serde(default, rename = "evidence", skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<EvidenceRecord>,
}

impl EvidenceFile {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn to_toml(&self) -> Result<String, ForumError> {
        toml::to_string(self)
            .map_err(|e| ForumError::SnapshotInvalid(format!("serialize evidence.toml: {e}")))
    }

    pub fn from_toml(s: &str) -> Result<Self, ForumError> {
        Ok(toml::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_kind_roundtrip() {
        for kind in [
            EvidenceKind::Commit,
            EvidenceKind::File,
            EvidenceKind::Benchmark,
            EvidenceKind::External,
        ] {
            let s = kind.to_string();
            let parsed: EvidenceKind = s.parse().unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn evidence_kind_unknown_returns_err() {
        assert!("bogus".parse::<EvidenceKind>().is_err());
    }

    #[test]
    fn evidence_serialize_omits_evidence_id() {
        let ev = Evidence {
            evidence_id: "sha123".into(),
            kind: EvidenceKind::Benchmark,
            ref_target: "bench/result.csv".into(),
            locator: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("evidence_id"));
        assert!(json.contains("benchmark"));
        assert!(json.contains("bench/result.csv"));
    }

    fn sample_record() -> EvidenceRecord {
        EvidenceRecord {
            id: "ev1".into(),
            kind: EvidenceKind::Commit,
            ref_target: "HEAD".into(),
            created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
            created_by: "human/alice".into(),
        }
    }

    #[test]
    fn evidence_record_round_trip() {
        let original = EvidenceFile {
            entries: vec![sample_record()],
        };
        let s = original.to_toml().unwrap();
        let parsed = EvidenceFile::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn evidence_record_round_trip_empty() {
        let original = EvidenceFile::default();
        let s = original.to_toml().unwrap();
        assert_eq!(EvidenceFile::from_toml(&s).unwrap(), original);
    }

    #[test]
    fn evidence_record_uses_spec_keys() {
        let s = EvidenceFile {
            entries: vec![sample_record()],
        }
        .to_toml()
        .unwrap();
        // SPEC-3.0 §2.3 keys.
        assert!(s.contains("id = "), "missing `id`: {s}");
        assert!(s.contains("kind = "), "missing `kind`: {s}");
        assert!(s.contains("ref = "), "missing `ref`: {s}");
        assert!(s.contains("created_at = "), "missing `created_at`: {s}");
        assert!(s.contains("created_by = "), "missing `created_by`: {s}");
        // Legacy field names MUST NOT appear.
        assert!(
            !s.contains("evidence_id = "),
            "unexpected `evidence_id`: {s}"
        );
        assert!(!s.contains("ref_target = "), "unexpected `ref_target`: {s}");
        assert!(!s.contains("locator = "), "unexpected `locator`: {s}");
    }

    #[test]
    fn evidence_record_rejects_legacy_field_names() {
        let bad = r#"
            [[evidence]]
            evidence_id = "ev1"
            kind = "commit"
            ref_target = "HEAD"
            created_at = "2026-05-03T00:00:00Z"
            created_by = "human/alice"
        "#;
        let err = EvidenceFile::from_toml(bad).unwrap_err();
        assert!(matches!(err, ForumError::Toml(_)));
    }
}
