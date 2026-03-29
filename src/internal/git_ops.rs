use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::config::CommitIdentity;
use super::error::{ForumError, ForumResult};

/// Thin subprocess wrapper for git plumbing commands.
pub struct GitOps {
    root: PathBuf,
    /// Optional override for git commit author/committer on forum commits.
    commit_identity: Option<CommitIdentity>,
    /// Default actor ID from local config (set during init).
    default_actor: Option<String>,
}

impl GitOps {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            commit_identity: None,
            default_actor: None,
        }
    }

    /// Set the commit identity used for forum commits.
    pub fn set_commit_identity(&mut self, identity: CommitIdentity) {
        self.commit_identity = Some(identity);
    }

    /// Set the default actor ID from local config.
    pub fn set_default_actor(&mut self, actor: String) {
        self.default_actor = Some(actor);
    }

    /// Get the configured default actor ID, if any.
    pub fn default_actor(&self) -> Option<&str> {
        self.default_actor.as_deref()
    }

    /// Discover the repository root from the current working directory.
    pub fn discover() -> ForumResult<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .output()?;
        if !output.status.success() {
            return Err(ForumError::Repo("not inside a git repository".into()));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Self::new(PathBuf::from(root)))
    }

    /// Resolve the actual `.git` directory path.
    ///
    /// In a normal repo this returns `<root>/.git`.
    /// In a worktree this returns the worktree-specific git dir
    /// (e.g. `/path/to/main/.git/worktrees/<name>`).
    pub fn git_dir(&self) -> ForumResult<PathBuf> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .output()?;
        if !output.status.success() {
            return Err(ForumError::Repo("cannot resolve git directory".into()));
        }
        let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = PathBuf::from(&git_dir);
        // --git-dir may return a relative path; canonicalize against repo root
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.root.join(path))
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Run a git command and return trimmed stdout.
    pub fn run(&self, args: &[&str]) -> ForumResult<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    /// Run a git command with data piped to stdin.
    pub fn run_with_stdin(&self, args: &[&str], data: &[u8]) -> ForumResult<String> {
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .expect("stdin must be available after Stdio::piped()")
            .write_all(data)?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    // ---- Object creation ----

    /// Write content as a blob and return its SHA.
    pub fn hash_object(&self, content: &[u8]) -> ForumResult<String> {
        self.run_with_stdin(&["hash-object", "-w", "--stdin"], content)
    }

    /// Create a tree with a single file entry.
    pub fn mktree_single(&self, filename: &str, blob_sha: &str) -> ForumResult<String> {
        let entry = format!("100644 blob {blob_sha}\t{filename}\n");
        self.run_with_stdin(&["mktree"], entry.as_bytes())
    }

    /// Create a commit from a tree, optional parents, and a message.
    ///
    /// When a `CommitIdentity` is configured, its name/email override the
    /// git config values for both author and committer fields.  Unset
    /// fields fall through to the normal git defaults.
    pub fn commit_tree(
        &self,
        tree_sha: &str,
        parents: &[&str],
        message: &str,
    ) -> ForumResult<String> {
        let mut args: Vec<String> = vec!["commit-tree".into(), tree_sha.into()];
        for p in parents {
            args.push("-p".into());
            args.push((*p).into());
        }
        args.push("-m".into());
        args.push(message.into());

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // If a commit identity is configured, set env vars on the command
        // directly instead of going through self.run().
        if let Some(ref id) = self.commit_identity {
            let mut cmd = Command::new("git");
            cmd.args(&arg_refs)
                .current_dir(&self.root)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_OBJECT_DIRECTORY")
                .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
            if let Some(ref name) = id.name {
                cmd.env("GIT_AUTHOR_NAME", name);
                cmd.env("GIT_COMMITTER_NAME", name);
            }
            if let Some(ref email) = id.email {
                cmd.env("GIT_AUTHOR_EMAIL", email);
                cmd.env("GIT_COMMITTER_EMAIL", email);
            }
            let output = cmd.output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(ForumError::Git(stderr));
            }
            Ok(String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string())
        } else {
            self.run(&arg_refs)
        }
    }

    // ---- Ref management ----

    pub fn update_ref(&self, refname: &str, sha: &str) -> ForumResult<()> {
        self.run(&["update-ref", refname, sha])?;
        Ok(())
    }

    /// Atomically update a ref only if its current value matches `old_sha`.
    ///
    /// Uses `git update-ref <ref> <new> <old>` for compare-and-swap.
    /// Returns ForumError::Git if the ref has been updated by another writer.
    pub fn update_ref_cas(&self, refname: &str, new_sha: &str, old_sha: &str) -> ForumResult<()> {
        self.run(&["update-ref", refname, new_sha, old_sha])
            .map_err(|_| {
                ForumError::Git(format!(
                    "concurrent write conflict on {refname}: expected {old_sha} but ref was updated by another writer. Retry your command."
                ))
            })?;
        Ok(())
    }

    /// Create a ref that must not already exist.
    ///
    /// Uses `git update-ref <ref> <new> 0{40}` to ensure the ref is new.
    pub fn create_ref(&self, refname: &str, sha: &str) -> ForumResult<()> {
        let zero = "0000000000000000000000000000000000000000";
        self.run(&["update-ref", refname, sha, zero]).map_err(|_| {
            ForumError::Git(format!(
                "ref {refname} already exists; concurrent create conflict"
            ))
        })?;
        Ok(())
    }

    /// Delete a ref.
    pub fn delete_ref(&self, refname: &str) -> ForumResult<()> {
        self.run(&["update-ref", "-d", refname])?;
        Ok(())
    }

    /// Query remote refs without fetching. Returns `Vec<(refname, sha)>`.
    pub fn ls_remote(&self, remote: &str, pattern: &str) -> ForumResult<Vec<(String, String)>> {
        let output = self.run(&["ls-remote", remote, pattern])?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        Ok(output
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let sha = parts.next()?.to_string();
                let refname = parts.next()?.to_string();
                Some((refname, sha))
            })
            .collect())
    }

    /// Check whether `maybe_ancestor` is an ancestor of `descendant`.
    pub fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> ForumResult<bool> {
        match self.run(&["merge-base", "--is-ancestor", maybe_ancestor, descendant]) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Resolve a ref to a commit SHA. Returns None if the ref doesn't exist.
    pub fn resolve_ref(&self, refname: &str) -> ForumResult<Option<String>> {
        match self.run(&["rev-parse", "--verify", refname]) {
            Ok(sha) => Ok(Some(sha)),
            Err(_) => Ok(None),
        }
    }

    /// Resolve a revision expression to a canonical commit OID.
    pub fn resolve_commit(&self, rev: &str) -> ForumResult<String> {
        let revspec = format!("{rev}^{{commit}}");
        self.run(&["rev-parse", "--verify", &revspec])
            .map_err(|_| ForumError::Repo(format!("revision '{rev}' does not resolve to a commit")))
    }

    /// List all ref names under a given prefix.
    pub fn list_refs(&self, prefix: &str) -> ForumResult<Vec<String>> {
        match self.run(&["for-each-ref", "--format=%(refname)", prefix]) {
            Ok(s) if s.is_empty() => Ok(vec![]),
            Ok(s) => Ok(s.lines().map(|l| l.to_string()).collect()),
            Err(_) => Ok(vec![]),
        }
    }

    /// List all refs under a prefix with their object names (SHAs).
    /// Returns `Vec<(refname, sha)>`.
    pub fn list_refs_with_shas(&self, prefix: &str) -> ForumResult<Vec<(String, String)>> {
        match self.run(&["for-each-ref", "--format=%(refname) %(objectname)", prefix]) {
            Ok(s) if s.is_empty() => Ok(vec![]),
            Ok(s) => Ok(s
                .lines()
                .filter_map(|l| {
                    let mut parts = l.splitn(2, ' ');
                    let refname = parts.next()?.to_string();
                    let sha = parts.next()?.to_string();
                    Some((refname, sha))
                })
                .collect()),
            Err(_) => Ok(vec![]),
        }
    }

    // ---- Reading ----

    /// List commits reachable from `start_ref`, newest first.
    pub fn rev_list(&self, start_ref: &str) -> ForumResult<Vec<String>> {
        let output = self.run(&["rev-list", start_ref])?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        Ok(output.lines().map(|l| l.to_string()).collect())
    }

    /// Read a file from a commit's tree (e.g. `<sha>:event.json`).
    pub fn show_file(&self, commit_sha: &str, path: &str) -> ForumResult<String> {
        let spec = format!("{commit_sha}:{path}");
        self.run(&["cat-file", "-p", &spec])
    }

    /// Run `git diff --no-index` between two files.
    ///
    /// Unlike normal git commands, `git diff --no-index` exits with status 1
    /// when differences are found (normal success case). This helper treats
    /// exit codes 0 (no diff) and 1 (diff found) as success, and only
    /// considers exit code >= 2 as an error.
    pub fn diff_no_index(
        &self,
        old_file: &str,
        new_file: &str,
        extra_args: &[&str],
    ) -> ForumResult<String> {
        let mut args = vec!["diff", "--no-index"];
        args.extend_from_slice(extra_args);
        args.push(old_file);
        args.push(new_file);
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .output()?;
        let code = output.status.code().unwrap_or(2);
        if code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }
}
