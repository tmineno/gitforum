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
}

/// Minimal local config stored in `.git/forum/local.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct LocalConfig {
    pub default_actor: Option<String>,
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
    fn load_local_config_missing_file_returns_default() {
        let paths = RepoPaths::from_repo_root(Path::new("/nonexistent"));
        let cfg = load_local_config(&paths).unwrap();
        assert!(cfg.default_actor.is_none());
    }
}
