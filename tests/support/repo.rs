use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// An isolated Git repository for integration tests.
///
/// - Creates a fresh temp directory with `git init`.
/// - Sets `user.name` / `user.email` locally.
/// - Isolates from global/system Git config via env vars.
pub struct TestRepo {
    _dir: TempDir,
    path: PathBuf,
}

impl TestRepo {
    /// Create a new isolated test repo.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path().to_path_buf();

        // git init
        let status = Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .envs(isolation_env())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git init failed");
        assert!(status.status.success(), "git init failed");

        // Set local user config
        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let status = Command::new("git")
                .args(["config", key, val])
                .current_dir(&path)
                .envs(isolation_env())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .output()
                .expect("git config failed");
            assert!(status.status.success());
        }

        Self { _dir: dir, path }
    }

    /// Root path of the test repo.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Env vars that isolate a git command from the host's global/system config.
fn isolation_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_is_git_repo() {
        let repo = TestRepo::new();
        assert!(repo.path().join(".git").is_dir());
    }
}
