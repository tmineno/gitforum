use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::error::{ForumError, ForumResult};

/// Paths derived from a git-forum repository root.
pub struct RepoPaths {
    /// `.forum/` — shared config committed to the repo.
    pub dot_forum: PathBuf,
    /// `.git/forum/` — local-only runtime data.
    pub git_forum: PathBuf,
}

impl RepoPaths {
    pub fn from_repo_root(root: &Path) -> Self {
        Self {
            dot_forum: root.join(".forum"),
            git_forum: root.join(".git").join("forum"),
        }
    }

    /// Build paths using an explicit git directory (worktree-safe).
    ///
    /// `git_dir` should come from `git rev-parse --git-dir`, which
    /// returns the correct path for both normal repos and worktrees.
    pub fn from_repo_root_and_git_dir(root: &Path, git_dir: &Path) -> Self {
        Self {
            dot_forum: root.join(".forum"),
            git_forum: git_dir.join("forum"),
        }
    }
}

/// Identity used for git commit author/committer on forum commits.
///
/// Resolution order (highest wins):
/// 1. Actor-level override from `.forum/actors.toml` (future)
/// 2. `[commit_identity]` in `.git/forum/local.toml`
/// 3. Git config `user.name` / `user.email` (default)
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CommitIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// Minimal local config stored in `.git/forum/local.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct LocalConfig {
    pub default_actor: Option<String>,
    pub commit_identity: Option<CommitIdentity>,
}

/// Load local config. Returns default if file doesn't exist.
pub fn load_local_config(paths: &RepoPaths) -> ForumResult<LocalConfig> {
    let path = paths.git_forum.join("local.toml");
    if !path.exists() {
        return Ok(LocalConfig::default());
    }
    let text = std::fs::read_to_string(&path)?;
    toml::from_str(&text).map_err(|e| ForumError::Config(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn repo_paths_construction() {
        let paths = RepoPaths::from_repo_root(Path::new("/tmp/repo"));
        assert_eq!(paths.dot_forum, Path::new("/tmp/repo/.forum"));
        assert_eq!(paths.git_forum, Path::new("/tmp/repo/.git/forum"));
    }

    #[test]
    fn repo_paths_with_explicit_git_dir() {
        // Simulates a worktree where git dir is separate from repo root
        let paths = RepoPaths::from_repo_root_and_git_dir(
            Path::new("/tmp/worktree"),
            Path::new("/tmp/main/.git/worktrees/worktree"),
        );
        assert_eq!(paths.dot_forum, Path::new("/tmp/worktree/.forum"));
        assert_eq!(
            paths.git_forum,
            Path::new("/tmp/main/.git/worktrees/worktree/forum")
        );
    }

    #[test]
    fn load_local_config_missing_file_returns_default() {
        let paths = RepoPaths::from_repo_root(Path::new("/nonexistent"));
        let cfg = load_local_config(&paths).unwrap();
        assert!(cfg.default_actor.is_none());
        assert!(cfg.commit_identity.is_none());
    }

    #[test]
    fn commit_identity_deserializes_from_toml() {
        let toml_str = r#"
[commit_identity]
name = "Forum Bot"
email = "bot@forum.local"
"#;
        let cfg: LocalConfig = toml::from_str(toml_str).unwrap();
        let id = cfg.commit_identity.unwrap();
        assert_eq!(id.name.as_deref(), Some("Forum Bot"));
        assert_eq!(id.email.as_deref(), Some("bot@forum.local"));
    }

    #[test]
    fn commit_identity_partial_name_only() {
        let toml_str = r#"
[commit_identity]
name = "Pseudonym"
"#;
        let cfg: LocalConfig = toml::from_str(toml_str).unwrap();
        let id = cfg.commit_identity.unwrap();
        assert_eq!(id.name.as_deref(), Some("Pseudonym"));
        assert!(id.email.is_none());
    }

    #[test]
    fn commit_identity_absent_when_section_missing() {
        let toml_str = r#"
default_actor = "human/alice"
"#;
        let cfg: LocalConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.commit_identity.is_none());
    }
}
