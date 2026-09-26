# ADR-015: Snapshot reads go through one `git cat-file --batch` per `GitOps`

Status: Accepted
Date: 2026-09-26
Spec: `doc/spec/LARGE-FORUM-READS.md`

## Context

git-forum talks to git by running plumbing commands as subprocesses
(`doc/spec/archive/SPEC.md`: "subprocess calls to `git` plumbing
commands … No libgit2 dependency"). Every read of a snapshot file is its
own process: `read_snapshot_at` runs one `ls-tree -r` and then one
`cat-file -p` per file. The number of processes for any command that reads
every thread therefore grows with threads × files.

On a copy of the openlt-rs forum refs (690 threads, 2,758 nodes; release
build of `4bfc137`; machine shared with other work):

- `ls` took 23.4 s and started 9,047 git processes.
- The TUI took 7.1 s to show the list and started 8,360.
- `node show` took 2.3–2.8 s and started 963.

The TUI also re-reads every thread whenever any thread ref changes, on the
thread that draws the screen.

## Decision

Keep git as a subprocess, but read objects through one long-lived
`git cat-file --batch` process per `GitOps`:

- **When it runs**: the process is started on the first read. `GitOps`
  sends each read to it, one request at a time, under a lock. On drop,
  `GitOps` closes the process's stdin and waits for it to exit.
- **What goes through it**: `show_file`, `show_file_bytes` and a new
  `list_tree_files` (the same output as
  `ls-tree -r --full-tree --name-only`). So `read_snapshot_at`,
  `NodeIdIndex::build` and every caller of them read without starting
  processes.
- **Tree parsing**: trees come back in git's binary format and are parsed
  here. The object name length (20 or 32 bytes) is taken from the hex
  object name in each reply, not assumed.
- **Fallback**: when the batch process answers `missing`, that one read is
  retried with a one-shot `cat-file -p`. Correctness therefore does not
  depend on how a running `cat-file --batch` sees objects written after it
  started.
- **Counter**: every git process `GitOps` starts is counted
  (`spawned_processes`), so tests can assert process counts.
- **Callers**: `list_thread_states` takes thread tips from `for-each-ref`
  instead of resolving each ref. The TUI list refresh reuses rows keyed by
  (thread id, tip SHA, published).

## Consequences

- A command that reads all threads starts the same number of git processes
  whatever the thread count. The remaining per-read cost is one pipe round
  trip.
- `GitOps` now owns a child process: it needs a lock and a `Drop`, and it
  can no longer be a plain value type.
- git-forum parses git's binary tree format itself. This is a small, stable
  format, but it is code git used to run for us.
- In a corrupt repository, the message for a missing object changes (the
  kind of error stays `ForumError::Git`).
- Writes still start one process per plumbing step. That cost is per thread
  touched, not per thread in the forum.

## Alternatives

- **gix (gitoxide) in process.**
  - Would remove process starts for reads and writes alike.
  - Adds a large dependency tree and reverses the subprocess stance.
  - Moves every git interaction onto a new code path at once.
  - Worth revisiting if writes become the bottleneck.
- **libgit2 (`git2`).** Same gains, plus a C dependency. Excluded by the
  existing stance.
- **`git cat-file --batch-command`** (git 2.36+). Allows explicit flushing,
  which a request-per-reply protocol does not need. It would also raise the
  minimum git version.
- **Keep per-file processes and cache parsed snapshots on disk.**
  - This is the SQLite index removed in task `913c4s9v`.
  - A cache on disk has to be invalidated.
  - Reads keyed by commit SHA need no invalidation.
- **Only make the TUI refresh incremental.**
  - Fixes repeated refreshes.
  - Leaves `ls`, TUI start-up and `node show` proportional to the thread
    count.

## Exit criteria

- **Done**: the acceptance tests in `doc/spec/LARGE-FORUM-READS.md`
  pass and the before/after measurement is recorded there.
- **Revisit** if a git version changes `cat-file --batch` output in a way
  the parser rejects, or if write-heavy commands become the reported
  bottleneck (then weigh gix).
