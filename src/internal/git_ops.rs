use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::error::{ForumError, ForumResult};

/// Thin subprocess wrapper for git plumbing commands.
pub struct GitOps {
    root: PathBuf,
}

impl GitOps {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Discover the repository root from the current working directory.
    pub fn discover() -> ForumResult<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !output.status.success() {
            return Err(ForumError::Repo("not inside a git repository".into()));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Self::new(PathBuf::from(root)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Run a git command and return trimmed stdout.
    pub fn run(&self, args: &[&str]) -> ForumResult<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
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
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(data)?;
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
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&refs)
    }

    // ---- Ref management ----

    pub fn update_ref(&self, refname: &str, sha: &str) -> ForumResult<()> {
        self.run(&["update-ref", refname, sha])?;
        Ok(())
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
}
