//! One long-lived `git cat-file --batch` process for reading objects
//! (ADR-015, `doc/spec/LARGE-FORUM-READS.md`).
//!
//! Starting a git process per file made every command that reads all
//! threads start a process per snapshot file. [`BatchReader`] keeps one
//! process and sends it one object name per line. `GitOps` owns it and
//! reads the old way whenever it cannot answer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// An object read through the batch process.
pub(super) struct Object {
    /// `blob`, `tree`, `commit` or `tag`.
    pub(super) kind: String,
    /// Bytes in an object id of this repository (20 for SHA-1, 32 for
    /// SHA-256), taken from the hex id in the reply.
    id_len: usize,
    pub(super) data: Vec<u8>,
}

/// A running `git cat-file --batch`.
pub(super) struct BatchReader {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl BatchReader {
    /// Start `git cat-file --batch` from `cmd`, a git command already set up
    /// for the repository.
    pub(super) fn start(mut cmd: Command) -> io::Result<Self> {
        let mut child = cmd
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);
        match stdout {
            Some(stdout) => Ok(Self {
                child,
                stdin,
                stdout,
            }),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                Err(io::Error::other("cat-file --batch has no stdout"))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Read the object `name` names (`<commit>:<path>`, `<commit>^{tree}`
    /// or an object id).
    ///
    /// `Ok(None)` when git answers that no such object exists (or the name
    /// is ambiguous). `Err` when the process is gone or the reply is not in
    /// the documented shape; the reader must not be used after that, since
    /// the replies may be out of step with the requests.
    pub(super) fn read(&mut self, name: &str) -> io::Result<Option<Object>> {
        if name.contains('\n') {
            return Ok(None);
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("cat-file --batch stdin is closed"))?;
        stdin.write_all(name.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;

        let mut header = String::new();
        if self.stdout.read_line(&mut header)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "cat-file --batch exited",
            ));
        }
        let header = header.strip_suffix('\n').unwrap_or(&header);
        if header.ends_with(" missing") || header.ends_with(" ambiguous") {
            return Ok(None);
        }
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected cat-file --batch reply: {header:?}"),
            )
        };
        let mut fields = header.split(' ');
        let (Some(id), Some(kind), Some(size), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(invalid());
        };
        let size: usize = size.parse().map_err(|_| invalid())?;
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let mut data = vec![0; size];
        self.stdout.read_exact(&mut data)?;
        let mut end = [0u8; 1];
        self.stdout.read_exact(&mut end)?;
        if end != *b"\n" {
            return Err(invalid());
        }
        Ok(Some(Object {
            kind: kind.to_string(),
            id_len: id.len() / 2,
            data,
        }))
    }
}

impl Drop for BatchReader {
    fn drop(&mut self) {
        // cat-file exits once its stdin closes. It could instead be blocked
        // writing a reply nobody reads (after an error mid-reply), so it is
        // also killed; it only reads, so nothing is lost.
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One entry of a tree object.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TreeEntry {
    /// Octal mode as git writes it (`40000` for a subtree).
    pub(super) mode: String,
    pub(super) name: String,
    /// Hex object id.
    pub(super) id: String,
}

/// Parse a tree object: entries of `<mode> <name>\0<id bytes>`, where the
/// id is `id_len` raw bytes.
pub(super) fn tree_entries(data: &[u8], id_len: usize) -> io::Result<Vec<TreeEntry>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "malformed tree object");
    let mut entries = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|&b| b == b' ').ok_or_else(invalid)?;
        let mode = std::str::from_utf8(&rest[..space]).map_err(|_| invalid())?;
        rest = &rest[space + 1..];
        let nul = rest.iter().position(|&b| b == 0).ok_or_else(invalid)?;
        let name = String::from_utf8_lossy(&rest[..nul]).into_owned();
        rest = &rest[nul + 1..];
        if rest.len() < id_len {
            return Err(invalid());
        }
        let id = rest[..id_len].iter().map(|b| format!("{b:02x}")).collect();
        rest = &rest[id_len..];
        entries.push(TreeEntry {
            mode: mode.to_string(),
            name,
            id,
        });
    }
    Ok(entries)
}

/// Every file path in `commit`'s tree, from the root, in the order
/// `git ls-tree -r --full-tree --name-only <commit>` prints them: subtrees
/// are walked in place, and everything that is not a subtree (files,
/// symlinks, submodule commits) is listed.
///
/// `Ok(None)` when an object is missing or of the wrong kind.
pub(super) fn list_files(
    reader: &mut BatchReader,
    commit: &str,
) -> io::Result<Option<Vec<String>>> {
    let mut files = Vec::new();
    let found = walk(reader, &format!("{commit}^{{tree}}"), "", &mut files)?;
    Ok(found.then_some(files))
}

fn walk(
    reader: &mut BatchReader,
    tree: &str,
    prefix: &str,
    files: &mut Vec<String>,
) -> io::Result<bool> {
    let Some(object) = reader.read(tree)? else {
        return Ok(false);
    };
    if object.kind != "tree" {
        return Ok(false);
    }
    for entry in tree_entries(&object.data, object.id_len)? {
        let path = format!("{prefix}{}", entry.name);
        if entry.mode == "40000" {
            if !walk(reader, &entry.id, &format!("{path}/"), files)? {
                return Ok(false);
            }
        } else {
            files.push(path);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_bytes(mode: &str, name: &str, id: &[u8]) -> Vec<u8> {
        let mut out = format!("{mode} {name}\0").into_bytes();
        out.extend_from_slice(id);
        out
    }

    #[test]
    fn tree_entries_read_sha1_and_sha256_ids() {
        for id_len in [20, 32] {
            let a: Vec<u8> = (0..id_len as u8).collect();
            let b = vec![0xab; id_len];
            let mut data = entry_bytes("100644", "thread.toml", &a);
            data.extend(entry_bytes("40000", "nodes", &b));
            let entries = tree_entries(&data, id_len).unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].mode, "100644");
            assert_eq!(entries[0].name, "thread.toml");
            assert_eq!(entries[0].id.len(), id_len * 2);
            assert!(entries[0].id.starts_with("000102"));
            assert_eq!(entries[1].mode, "40000");
            assert_eq!(entries[1].id, "ab".repeat(id_len));
        }
    }

    #[test]
    fn tree_entries_reject_a_cut_off_tree() {
        let data = entry_bytes("100644", "thread.toml", &[1; 20]);
        assert!(tree_entries(&data[..data.len() - 1], 20).is_err());
        assert!(tree_entries(b"100644 no-nul", 20).is_err());
    }
}
