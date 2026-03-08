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

    #[error("git error: {0}")]
    Git(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
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
