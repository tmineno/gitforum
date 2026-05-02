//! Throttled emission of policy / migration lint warnings (#6k7hq482).
//!
//! ## Why this exists
//!
//! Two ergonomic problems with the previous direct `eprintln!` flow:
//!
//! 1. **Absolute paths leaked the user's home directory layout.** The
//!    policy.toml warning prefix was rendered with `path.display()`, which
//!    printed `/home/<user>/.../policy.toml:8:` into stderr. That output
//!    routinely landed in forum events, screenshots, and pipeline logs.
//!
//! 2. **The warning fired on every command.** A single legacy predicate in
//!    `policy.toml` produced one warning per `git forum` invocation. Every
//!    call had to be `| tail`-ed to stay readable until the user edited
//!    their policy.
//!
//! The emitter centralizes formatting and throttling so all lint sites get
//! both fixes for free.
//!
//! ## Suppression model
//!
//! - **In-process**: a `(kind, path, line)` triple emits at most once per
//!   `LintEmitter`. Subsequent calls with the same key are dropped.
//! - **On-disk**: when the emitter has a cache path, the same key is
//!   suppressed across processes for [`SUPPRESS_WINDOW`]. The cache lives
//!   at `.git/forum/lints-seen.toml` (per-clone, not committed).
//! - **Escape hatch**: `GIT_FORUM_LINT_VERBOSE=1` disables both layers.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::config::RepoPaths;

/// How long an on-disk acknowledgement suppresses a repeat warning.
pub const SUPPRESS_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// Where formatted warnings go when the throttle lets them through.
enum LintSink {
    /// Production: write to the process's stderr.
    Stderr,
    /// Tests: append to a buffer the test can later inspect.
    Buffer(Mutex<Vec<String>>),
}

/// Centralized throttled lint emitter.
///
/// Construct via [`LintEmitter::new_for_paths`] (production), or
/// [`LintEmitter::new_capturing`] / [`LintEmitter::in_memory`] (tests).
pub struct LintEmitter {
    state: Mutex<EmitterState>,
    sink: LintSink,
    verbose: bool,
    repo_root: Option<PathBuf>,
    cache_path: Option<PathBuf>,
    suppress_window: Duration,
}

#[derive(Default)]
struct EmitterState {
    process_seen: HashSet<u64>,
    disk_loaded: bool,
    disk_seen: HashMap<u64, u64>,
}

impl LintEmitter {
    /// Production emitter wired to a real repo. Writes to stderr; uses the
    /// repo's `.git/forum/lints-seen.toml` for cross-process throttling.
    pub fn new_for_paths(paths: &RepoPaths) -> Self {
        let repo_root = paths.dot_forum.parent().map(Path::to_path_buf);
        Self {
            state: Mutex::new(EmitterState::default()),
            sink: LintSink::Stderr,
            verbose: env_verbose(),
            repo_root,
            cache_path: Some(paths.git_forum.join("lints-seen.toml")),
            suppress_window: SUPPRESS_WINDOW,
        }
    }

    /// Test emitter that captures formatted warnings in memory. The
    /// `repo_root` controls how source paths are displayed (paths inside
    /// it become repo-relative; paths outside fall back to absolute).
    pub fn new_capturing(repo_root: Option<PathBuf>) -> Self {
        Self {
            state: Mutex::new(EmitterState::default()),
            sink: LintSink::Buffer(Mutex::new(Vec::new())),
            verbose: env_verbose(),
            repo_root,
            cache_path: None,
            suppress_window: SUPPRESS_WINDOW,
        }
    }

    /// Convenience: capturing emitter with no repo root (every path
    /// renders absolute). Suitable for unit tests that don't care about
    /// path display.
    pub fn in_memory() -> Self {
        Self::new_capturing(None)
    }

    /// Override the on-disk cache location (tests).
    pub fn with_cache_path(mut self, cache_path: PathBuf) -> Self {
        self.cache_path = Some(cache_path);
        self
    }

    /// Override the suppression window (tests).
    pub fn with_suppress_window(mut self, window: Duration) -> Self {
        self.suppress_window = window;
        self
    }

    /// Force verbose mode on or off, ignoring the env var (tests).
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Drain the captured buffer. Returns `None` for stderr-backed
    /// emitters; production callers don't need this.
    pub fn captured(&self) -> Option<Vec<String>> {
        match &self.sink {
            LintSink::Buffer(b) => Some(b.lock().unwrap().clone()),
            LintSink::Stderr => None,
        }
    }

    /// Emit a single lint warning, subject to throttling.
    ///
    /// `kind` is the deprecation/lint identifier (e.g.
    /// `"at_least_one_summary"`); it joins `(source, line)` to form the
    /// suppression key so distinct lints don't shadow each other.
    ///
    /// `source` + `line` are an optional file:line prefix. When set, the
    /// path is rendered repo-relative if the emitter knows the repo root
    /// and the path falls under it; otherwise it falls back to absolute
    /// with a `(outside repo root)` note.
    ///
    /// `message` is the human-readable warning body, without the
    /// `warning: ` prefix or the `file:line:` prefix.
    pub fn emit(&self, kind: &str, source: Option<&Path>, line: Option<usize>, message: &str) {
        let hash = compute_hash(kind, source, line);
        if self.verbose {
            self.write(source, line, message);
            return;
        }

        let mut state = self.state.lock().unwrap();
        if state.process_seen.contains(&hash) {
            return;
        }

        if let Some(cache) = &self.cache_path {
            if !state.disk_loaded {
                state.disk_seen = read_disk_cache(cache);
                state.disk_loaded = true;
            }
            if let Some(&ts) = state.disk_seen.get(&hash) {
                let now = unix_now();
                if now.saturating_sub(ts) < self.suppress_window.as_secs() {
                    state.process_seen.insert(hash);
                    return;
                }
            }
        }

        self.write(source, line, message);
        state.process_seen.insert(hash);

        if let Some(cache) = &self.cache_path {
            state.disk_seen.insert(hash, unix_now());
            // Cache write is best-effort: lint throttling is an ergonomic
            // optimization, not a correctness gate. A read-only filesystem
            // shouldn't take down the command.
            let _ = write_disk_cache(cache, &state.disk_seen);
        }
    }

    fn write(&self, source: Option<&Path>, line: Option<usize>, message: &str) {
        let prefix = match (source, line) {
            (Some(p), Some(l)) => format!("{}:{l}: ", self.format_path(p)),
            (Some(p), None) => format!("{}: ", self.format_path(p)),
            _ => String::new(),
        };
        let formatted = format!("warning: {prefix}{message}");
        match &self.sink {
            LintSink::Stderr => eprintln!("{formatted}"),
            LintSink::Buffer(b) => b.lock().unwrap().push(formatted),
        }
    }

    fn format_path(&self, p: &Path) -> String {
        format_path_repo_relative(p, self.repo_root.as_deref())
    }
}

/// Render `path` for display in a lint warning.
///
/// - With `repo_root = Some(root)` and `path` inside `root`: return the
///   relative form (`.forum/policy.toml`).
/// - With `repo_root = Some(root)` and `path` outside `root`: return
///   absolute and inline `(outside repo root)` so the user can see why
///   the prefix wasn't stripped. The note sits on a single line so
///   `:`-splitting pipeline tools still work.
/// - With `repo_root = None`: return the path as-is. No note — there's
///   no repo to be relative to, so absolute is the most accurate answer
///   we can give.
///
/// Used by [`LintEmitter`] internally and by other diagnostic sites
/// (e.g. `internal::migrate`) that need the same path-scoping rule.
pub fn format_path_repo_relative(path: &Path, repo_root: Option<&Path>) -> String {
    match repo_root {
        Some(root) => match path.strip_prefix(root) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => format!("{} (outside repo root)", path.display()),
        },
        None => path.display().to_string(),
    }
}

fn compute_hash(kind: &str, source: Option<&Path>, line: Option<usize>) -> u64 {
    let mut h = DefaultHasher::new();
    kind.hash(&mut h);
    source
        .map(|p| p.to_string_lossy().into_owned())
        .hash(&mut h);
    line.unwrap_or(0).hash(&mut h);
    h.finish()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn env_verbose() -> bool {
    match std::env::var("GIT_FORUM_LINT_VERBOSE") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

fn read_disk_cache(path: &Path) -> HashMap<u64, u64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return HashMap::new();
    };
    let Some(table) = value.get("seen").and_then(|v| v.as_table()) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (k, v) in table {
        let Ok(hash) = k.parse::<u64>() else {
            continue;
        };
        let Some(ts) = v.as_integer().and_then(|i| u64::try_from(i).ok()) else {
            continue;
        };
        out.insert(hash, ts);
    }
    out
}

fn write_disk_cache(path: &Path, seen: &HashMap<u64, u64>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = String::from(
        "# git-forum lint suppression cache (per-clone, not committed).\n\
         # Format: each entry is `<hash> = <unix-seconds-acknowledged>`.\n\
         # Set GIT_FORUM_LINT_VERBOSE=1 to bypass and re-emit every warning.\n\
         [seen]\n",
    );
    let mut entries: Vec<(&u64, &u64)> = seen.iter().collect();
    entries.sort_by_key(|(h, _)| **h);
    for (hash, ts) in entries {
        body.push_str(&format!("\"{hash}\" = {ts}\n"));
    }
    std::fs::write(path, body)
}

static GLOBAL: OnceLock<LintEmitter> = OnceLock::new();

/// Install the process-wide emitter. Idempotent — first call wins.
pub fn install(emitter: LintEmitter) {
    let _ = GLOBAL.set(emitter);
}

/// Return the installed emitter, or a default in-memory one if `install`
/// hasn't been called (test/library use).
pub fn current() -> &'static LintEmitter {
    GLOBAL.get_or_init(LintEmitter::in_memory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn emits_repo_relative_path_when_inside_repo() {
        let root = PathBuf::from("/tmp/repo");
        let emitter = LintEmitter::new_capturing(Some(root.clone()));
        let policy_path = root.join(".forum/policy.toml");

        emitter.emit("kind1", Some(&policy_path), Some(8), "boom");

        let captured = emitter.captured().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], "warning: .forum/policy.toml:8: boom");
    }

    #[test]
    fn falls_back_to_absolute_when_outside_repo() {
        let emitter = LintEmitter::new_capturing(Some(PathBuf::from("/tmp/repo")));
        let outside = PathBuf::from("/etc/policy.toml");

        emitter.emit("kind1", Some(&outside), Some(2), "boom");

        let captured = emitter.captured().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0],
            "warning: /etc/policy.toml (outside repo root):2: boom"
        );
    }

    #[test]
    fn suppresses_repeat_in_same_process() {
        let emitter = LintEmitter::new_capturing(None);
        let p = PathBuf::from("/x/policy.toml");

        emitter.emit("kind1", Some(&p), Some(8), "first");
        emitter.emit("kind1", Some(&p), Some(8), "first");

        assert_eq!(emitter.captured().unwrap().len(), 1);
    }

    #[test]
    fn distinct_keys_emit_independently() {
        let emitter = LintEmitter::new_capturing(None);
        let p = PathBuf::from("/x/policy.toml");

        emitter.emit("kind1", Some(&p), Some(8), "a");
        emitter.emit("kind1", Some(&p), Some(9), "b"); // different line
        emitter.emit("kind2", Some(&p), Some(8), "c"); // different kind

        assert_eq!(emitter.captured().unwrap().len(), 3);
    }

    #[test]
    fn verbose_flag_disables_throttle() {
        let emitter = LintEmitter::new_capturing(None).with_verbose(true);
        let p = PathBuf::from("/x/policy.toml");

        emitter.emit("kind1", Some(&p), Some(8), "boom");
        emitter.emit("kind1", Some(&p), Some(8), "boom");
        emitter.emit("kind1", Some(&p), Some(8), "boom");

        assert_eq!(emitter.captured().unwrap().len(), 3);
    }

    #[test]
    fn on_disk_cache_suppresses_across_processes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("lints-seen.toml");
        let p = PathBuf::from("/x/policy.toml");

        // First "process": emit and write cache.
        let emitter1 = LintEmitter::new_capturing(None).with_cache_path(cache.clone());
        emitter1.emit("kind1", Some(&p), Some(8), "boom");
        assert_eq!(emitter1.captured().unwrap().len(), 1);
        assert!(cache.exists());

        // Second "process": fresh in-process state, but disk cache hits.
        let emitter2 = LintEmitter::new_capturing(None).with_cache_path(cache.clone());
        emitter2.emit("kind1", Some(&p), Some(8), "boom");
        assert!(emitter2.captured().unwrap().is_empty());
    }

    #[test]
    fn on_disk_cache_expires_after_window() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("lints-seen.toml");
        let p = PathBuf::from("/x/policy.toml");

        let emitter1 = LintEmitter::new_capturing(None)
            .with_cache_path(cache.clone())
            .with_suppress_window(Duration::from_secs(0));
        emitter1.emit("kind1", Some(&p), Some(8), "boom");

        let emitter2 = LintEmitter::new_capturing(None)
            .with_cache_path(cache)
            .with_suppress_window(Duration::from_secs(0));
        emitter2.emit("kind1", Some(&p), Some(8), "boom");

        assert_eq!(emitter2.captured().unwrap().len(), 1);
    }

    #[test]
    fn verbose_bypasses_disk_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("lints-seen.toml");
        let p = PathBuf::from("/x/policy.toml");

        // Seed the disk cache with a recent acknowledgement.
        let seeder = LintEmitter::new_capturing(None).with_cache_path(cache.clone());
        seeder.emit("kind1", Some(&p), Some(8), "boom");

        // Verbose emitter should ignore the cache entry.
        let emitter = LintEmitter::new_capturing(None)
            .with_cache_path(cache)
            .with_verbose(true);
        emitter.emit("kind1", Some(&p), Some(8), "boom");
        emitter.emit("kind1", Some(&p), Some(8), "boom");

        assert_eq!(emitter.captured().unwrap().len(), 2);
    }

    #[test]
    fn no_source_emits_bare_message() {
        let emitter = LintEmitter::new_capturing(None);
        emitter.emit("kind1", None, None, "creation_rules.task: rewritten");

        let captured = emitter.captured().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], "warning: creation_rules.task: rewritten");
    }
}
