use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Utc};

use super::config::CommitIdentity;
use super::error::{ForumError, ForumResult};
use super::git_batch::{self, BatchReader};

/// Environment variables that would point git at another repository.
/// Every git process started here runs without them.
const GIT_REPO_ENV: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

/// A `git` command without the [`GIT_REPO_ENV`] variables.
fn git_command() -> Command {
    let mut cmd = Command::new("git");
    for var in GIT_REPO_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// Thin subprocess wrapper for git plumbing commands.
pub struct GitOps {
    root: PathBuf,
    /// Optional override for git commit author/committer on forum commits.
    commit_identity: Option<CommitIdentity>,
    /// Default actor ID from local config (set during init).
    default_actor: Option<String>,
    /// git commands run through [`GitOps::command`].
    spawned: AtomicUsize,
    /// The `git cat-file --batch` process reads go through (ADR-015).
    batch: Mutex<BatchSlot>,
}

/// The batch process, started on the first read.
#[derive(Default)]
struct BatchSlot {
    reader: Option<BatchReader>,
    /// Starts that failed and processes that died. At 2 the slot is off
    /// and every read goes the old way.
    failures: u8,
}

impl GitOps {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            commit_identity: None,
            default_actor: None,
            spawned: AtomicUsize::new(0),
            batch: Mutex::new(BatchSlot::default()),
        }
    }

    /// The number of git processes this value has started.
    ///
    /// - Preconditions: none.
    /// - Postconditions: counts every git command run by a method of this
    ///   value, including one whose process then failed; `discover` runs
    ///   before the value exists and is not counted.
    /// - Failure modes: none.
    /// - Side effects: none.
    pub fn spawned_processes(&self) -> usize {
        self.spawned.load(Ordering::Relaxed)
    }

    /// A `git` command in this repository, counted by
    /// [`GitOps::spawned_processes`].
    fn command(&self) -> Command {
        self.spawned.fetch_add(1, Ordering::Relaxed);
        let mut cmd = git_command();
        cmd.current_dir(&self.root);
        cmd
    }

    /// Run `read` on the batch process, starting it if needed.
    ///
    /// `None` means the caller must read the old way: the object is missing
    /// or of another kind (`read` answered `Ok(None)`), or the process is
    /// unusable. A process that fails is dropped and started again on the
    /// next attempt; after two failures the slot stays off.
    fn with_batch<T>(
        &self,
        mut read: impl FnMut(&mut BatchReader) -> std::io::Result<Option<T>>,
    ) -> Option<T> {
        let mut slot = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
        while slot.failures < 2 {
            if slot.reader.is_none() {
                match BatchReader::start(self.command()) {
                    Ok(reader) => slot.reader = Some(reader),
                    Err(_) => {
                        slot.failures += 1;
                        continue;
                    }
                }
            }
            let reader = slot.reader.as_mut().expect("started above");
            match read(reader) {
                Ok(found) => return found,
                Err(_) => {
                    slot.reader = None;
                    slot.failures += 1;
                }
            }
        }
        None
    }

    /// The content of the blob `<commit>:<path>`, through the batch process.
    fn batch_blob(&self, spec: &str) -> Option<Vec<u8>> {
        self.with_batch(|reader| {
            Ok(reader
                .read(spec)?
                .filter(|object| object.kind == "blob")
                .map(|object| object.data))
        })
    }

    /// The process id of the batch process, if one is running.
    #[cfg(test)]
    fn batch_pid(&self) -> Option<u32> {
        let slot = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
        slot.reader.as_ref().map(BatchReader::pid)
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
        let output = git_command()
            .args(["rev-parse", "--show-toplevel"])
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
        let output = self.command().args(["rev-parse", "--git-dir"]).output()?;
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
        let output = self.command().args(args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(annotate_git_stderr(&stderr)));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    /// Run a git command with data piped to stdin.
    pub fn run_with_stdin(&self, args: &[&str], data: &[u8]) -> ForumResult<String> {
        let mut child = self
            .command()
            .args(args)
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
            return Err(ForumError::Git(annotate_git_stderr(&stderr)));
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
            let mut cmd = self.command();
            cmd.args(&arg_refs);
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
                return Err(ForumError::Git(annotate_git_stderr(&stderr)));
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

    /// Get the author timestamp of a commit as a `DateTime<Utc>`.
    ///
    /// Accepts any revision expression (tag, branch, SHA).
    pub fn commit_timestamp(&self, rev: &str) -> ForumResult<DateTime<Utc>> {
        let sha = self.resolve_commit(rev)?;
        let iso = self.run(&["log", "--format=%aI", "-1", &sha])?;
        DateTime::parse_from_rfc3339(iso.trim())
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| ForumError::Git(format!("cannot parse timestamp for '{rev}': {e}")))
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
    ///
    /// Returns the file contents with trailing whitespace trimmed.
    /// Suitable for parsing structured formats (JSON, TOML) but
    /// destructive for content that may legitimately end with a
    /// newline (Markdown bodies, etc.) — use [`show_file_bytes`] for
    /// byte-exact reads.
    ///
    /// Reads through the batch process (ADR-015); when it cannot answer,
    /// runs `cat-file -p` as before, so results and errors are unchanged.
    pub fn show_file(&self, commit_sha: &str, path: &str) -> ForumResult<String> {
        let spec = format!("{commit_sha}:{path}");
        if let Some(data) = self.batch_blob(&spec) {
            return Ok(String::from_utf8_lossy(&data).trim_end().to_string());
        }
        self.run(&["cat-file", "-p", &spec])
    }

    /// Read a file from a commit's tree as raw bytes, preserving
    /// trailing whitespace and any binary content.
    ///
    /// Reads through the batch process (ADR-015); when it cannot answer,
    /// runs `cat-file -p` as before, so results and errors are unchanged.
    pub fn show_file_bytes(&self, commit_sha: &str, path: &str) -> ForumResult<Vec<u8>> {
        let spec = format!("{commit_sha}:{path}");
        if let Some(data) = self.batch_blob(&spec) {
            return Ok(data);
        }
        let output = self.command().args(["cat-file", "-p", &spec]).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(stderr));
        }
        Ok(output.stdout)
    }

    /// Every file path in `commit`'s tree.
    ///
    /// - Preconditions: `commit` names a commit (or tree) in this repository.
    /// - Postconditions: the paths `git ls-tree -r --full-tree --name-only
    ///   <commit>` prints, in the same order, relative to the tree root
    ///   whatever this value's root directory is. Names that `ls-tree`
    ///   would quote (non-ASCII, control characters) come back unquoted.
    /// - Failure modes: `ForumError::Git` from `ls-tree` when `commit` does
    ///   not name a tree.
    /// - Side effects: starts the batch process on first use (ADR-015);
    ///   when it cannot answer, runs `ls-tree` as before.
    pub fn list_tree_files(&self, commit: &str) -> ForumResult<Vec<String>> {
        if let Some(files) = self.with_batch(|reader| git_batch::list_files(reader, commit)) {
            return Ok(files);
        }
        let listing = self.run(&["ls-tree", "-r", "--full-tree", "--name-only", commit])?;
        Ok(listing.lines().map(String::from).collect())
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
        let output = self.command().args(&args).output()?;
        let code = output.status.code().unwrap_or(2);
        if code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ForumError::Git(annotate_git_stderr(&stderr)));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }
}

/// Ticket `1nqfgm8u`: when git stderr names a low-level
/// object/ref/temp-file write failure, prepend a forum-level hint that
/// (a) tells the operator the forum mutation never reached the thread
/// ref, (b) names the most likely cause (object database / repo
/// permissions, read-only file system, missing/full disk), and
/// (c) says retrying the same command is safe once the underlying
/// condition is fixed. Anything else passes through unchanged so we
/// don't bury unrelated git output.
fn annotate_git_stderr(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    let object_db_hit = lower.contains("unable to create temporary file")
        || lower.contains("read-only file system")
        || lower.contains("unable to add")
        || lower.contains("could not write")
        || lower.contains("error writing object")
        || lower.contains("no space left on device")
        || lower.contains("permission denied")
        || lower.contains("unable to write");
    if !object_db_hit {
        return stderr.to_string();
    }
    let mut out = String::new();
    out.push_str(
        "forum write failed before updating the thread ref.\n  \
         Git could not write to the object database or ref store.\n  \
         Check repository/object database write permissions, free disk space,\n  \
         and that the repo is not on a read-only file system, then retry the\n  \
         same command.\n  \
         The forum mutation has not been recorded; retrying is safe.\n  \
         underlying git error:\n",
    );
    for line in stderr.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    let trimmed = out.trim_end().to_string();
    trimmed
}

#[cfg(test)]
mod git_error_annotation_tests {
    use super::annotate_git_stderr;

    #[test]
    fn annotates_unable_to_create_temporary_file() {
        let stderr = "error: unable to create temporary file: Read-only file system\n\
                      fatal: Unable to add (null) to database";
        let out = annotate_git_stderr(stderr);
        assert!(
            out.contains("forum write failed before updating the thread ref"),
            "expected forum-level hint, got:\n{out}"
        );
        assert!(
            out.contains("retrying is safe"),
            "expected retry guidance, got:\n{out}"
        );
        assert!(
            out.contains("unable to create temporary file"),
            "expected raw git error to be preserved, got:\n{out}"
        );
    }

    #[test]
    fn annotates_no_space_left() {
        let stderr = "error: No space left on device";
        let out = annotate_git_stderr(stderr);
        assert!(out.contains("forum write failed"));
        assert!(out.contains("free disk space"));
    }

    #[test]
    fn passes_unrelated_errors_through_unchanged() {
        let stderr = "fatal: not a git repository";
        let out = annotate_git_stderr(stderr);
        assert_eq!(out, stderr);
    }
}

#[cfg(test)]
mod spawn_count_tests {
    use super::GitOps;

    #[test]
    fn every_git_command_is_counted() {
        let dir = tempfile::TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        assert_eq!(git.spawned_processes(), 0);
        git.run(&["--version"]).unwrap();
        git.run(&["--version"]).unwrap();
        // A failing command still started a process.
        assert!(git.run(&["rev-parse", "--verify", "HEAD"]).is_err());
        assert_eq!(git.spawned_processes(), 3);
    }
}

#[cfg(test)]
mod batch_tests {
    use super::GitOps;

    /// A repository holding a tree with `a.txt`; returns the tree. Reads
    /// take any tree-ish, so no commit (and no signing config) is needed.
    fn repo_with_tree(dir: &std::path::Path) -> (GitOps, String) {
        let git = GitOps::new(dir.to_path_buf());
        git.run(&["init", "-q"]).unwrap();
        let blob = git.hash_object(b"content\n").unwrap();
        let tree = git.mktree_single("a.txt", &blob).unwrap();
        (git, tree)
    }

    /// AT-8: dropping the `GitOps` ends its batch process.
    #[cfg(target_os = "linux")]
    #[test]
    fn dropping_gitops_ends_the_batch_process() {
        let dir = tempfile::TempDir::new().unwrap();
        let (git, tree) = repo_with_tree(dir.path());
        assert_eq!(git.batch_pid(), None);
        assert_eq!(git.show_file(&tree, "a.txt").unwrap(), "content");
        assert_eq!(git.list_tree_files(&tree).unwrap(), ["a.txt"]);
        let pid = git.batch_pid().expect("the read started the batch process");
        let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        assert!(proc_dir.exists());
        drop(git);
        assert!(!proc_dir.exists(), "batch process {pid} still exists");
    }

    /// A batch process that cannot start is tried twice, then reads go the
    /// old way without trying again (spec Failure modes 1).
    #[test]
    fn a_batch_process_that_cannot_start_is_given_up_after_two_tries() {
        let dir = tempfile::TempDir::new().unwrap();
        let git = GitOps::new(dir.path().join("missing"));
        assert!(git.show_file("HEAD", "a.txt").is_err());
        // Two failed starts, then `cat-file -p` the old way.
        assert_eq!(git.spawned_processes(), 3);
        assert!(git.show_file_bytes("HEAD", "a.txt").is_err());
        assert!(git.list_tree_files("HEAD").is_err());
        assert_eq!(git.spawned_processes(), 5);
        assert_eq!(git.batch_pid(), None);
    }
}
