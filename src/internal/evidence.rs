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
}
