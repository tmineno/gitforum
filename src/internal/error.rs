use std::fmt;

/// Top-level error type for git-forum.
#[derive(Debug, thiserror::Error)]
pub enum ForumError {
    #[error("repository error: {0}")]
    Repo(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("policy error: {0}")]
    Policy(String),

    #[error("state machine error: {0}")]
    StateMachine(String),

    /// SPEC-2.0 §13: state transition not allowed for thread's lifecycle.
    #[error("lifecycle state mismatch: {0}")]
    LifecycleStateMismatch(String),

    /// SPEC-2.0 §13: facet mutation in a state that doesn't allow it
    /// (e.g. setting `lifecycle` after the first `facet_set`).
    #[error("facet transition disallowed: {0}")]
    FacetTransitionDisallowed(String),

    /// SPEC-2.0 §13 / §2.3.5: tag string violates the grammar.
    #[error("invalid tag syntax: {0}")]
    InvalidTagSyntax(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// SPEC-3.0 §11: thread ref tip lacks `thread.toml`.
    #[error("snapshot missing: {0}")]
    SnapshotMissing(String),

    /// SPEC-3.0 §11: `schema_version` is absent or unsupported.
    #[error("snapshot schema unsupported: {0}")]
    SnapshotSchemaUnsupported(String),

    /// SPEC-3.0 §11: snapshot fields fail schema or grammar checks.
    #[error("snapshot invalid: {0}")]
    SnapshotInvalid(String),

    /// SPEC-3.0 §11: 3.0 command sees an unmigrated 1.x/2.x event chain.
    #[error("legacy event chain at thread ref; run `git forum migrate`")]
    LegacyEventChain,

    /// SPEC-3.0 §5/§10: concurrent create or stale-parent CAS write
    /// conflict on `refs/forum/threads/<id>`. Caller should re-read the
    /// latest snapshot and re-apply. Not in §11's enumerated table; the
    /// SPEC mandates the no-silent-overwrite semantics without naming
    /// the error.
    #[error("snapshot write conflict: {0}")]
    SnapshotWriteConflict(String),

    /// TOML deserialization error. Snapshot codecs use this directly so
    /// parser context (line/column) is preserved instead of being
    /// flattened into a `Config` or `Json` string.
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Result alias used throughout the crate.
pub type ForumResult<T> = Result<T, ForumError>;

/// Structured detail returned to the user on CLI failure.
///
/// Includes: what went wrong, which rule was violated, and how to fix it.
pub struct CliDiagnostic {
    pub reason: String,
    pub violated_rule: Option<String>,
    pub hint: Option<String>,
}

impl fmt::Display for CliDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}", self.reason)?;
        if let Some(rule) = &self.violated_rule {
            write!(f, "\n  violated: {rule}")?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = ForumError::Policy("no open objections required".into());
        assert_eq!(err.to_string(), "policy error: no open objections required");
    }

    #[test]
    fn diagnostic_display() {
        let diag = CliDiagnostic {
            reason: "transition denied".into(),
            violated_rule: Some("no_open_objections".into()),
            hint: Some("resolve all objections first".into()),
        };
        let s = diag.to_string();
        assert!(s.contains("transition denied"));
        assert!(s.contains("no_open_objections"));
        assert!(s.contains("resolve all objections first"));
    }
}
