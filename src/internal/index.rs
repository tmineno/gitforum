use std::path::Path;

use rusqlite::{params, Connection};

use super::error::{ForumError, ForumResult};
use super::thread::ThreadState;

/// A denormalized thread row cached in the local SQLite index.
#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub body: Option<String>,
    pub branch: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub open_objections: i64,
    pub open_actions: i64,
    pub has_summary: bool,
    pub tip_sha: String,
    pub updated_at: String,
}

/// A current node body that matched a search query.
#[derive(Debug, Clone)]
pub struct NodeSearchHit {
    pub node_id: String,
    pub node_type: String,
    pub status: String,
    pub body: String,
}

/// A thread result with optional matching node hits.
#[derive(Debug, Clone)]
pub struct SearchRow {
    pub thread: ThreadRow,
    pub node_hits: Vec<NodeSearchHit>,
}

/// Open (or create) the SQLite index at `path` and ensure the schema exists.
///
/// Preconditions: parent directory of `path` may or may not exist (created if absent).
/// Postconditions: schema is applied; connection is ready to use.
/// Failure modes: ForumError::Repo if SQLite fails.
/// Side effects: may create the parent directory and/or the database file.
pub fn open_db(path: &Path) -> ForumResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path).map_err(|e| ForumError::Repo(e.to_string()))?;
    // Restrict file permissions to owner-only on Unix (prevents inter-user reads
    // on shared systems, since the index contains full thread/node bodies).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    ensure_schema(&conn)?;
    Ok(conn)
}

fn ensure_schema(conn: &Connection) -> ForumResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS threads (
            id              TEXT PRIMARY KEY,
            kind            TEXT NOT NULL,
            status          TEXT NOT NULL,
            title           TEXT NOT NULL,
            body            TEXT,
            branch          TEXT,
            created_at      TEXT NOT NULL,
            created_by      TEXT NOT NULL,
            open_objections INTEGER NOT NULL DEFAULT 0,
            open_actions    INTEGER NOT NULL DEFAULT 0,
            has_summary     INTEGER NOT NULL DEFAULT 0,
            tip_sha         TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS nodes (
            id              TEXT PRIMARY KEY,
            thread_id       TEXT NOT NULL,
            node_type       TEXT NOT NULL,
            status          TEXT NOT NULL,
            body            TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS evidence (
            id              TEXT PRIMARY KEY,
            thread_id       TEXT NOT NULL,
            kind            TEXT NOT NULL,
            ref_target      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_evidence_ref ON evidence(ref_target);",
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    ensure_thread_branch_column(conn)?;
    ensure_thread_updated_at_column(conn)
}

fn ensure_thread_branch_column(conn: &Connection) -> ForumResult<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let mut has_branch = false;
    for col in cols {
        if col.map_err(|e| ForumError::Repo(e.to_string()))? == "branch" {
            has_branch = true;
            break;
        }
    }
    if !has_branch {
        conn.execute("ALTER TABLE threads ADD COLUMN branch TEXT", [])
            .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    Ok(())
}

fn ensure_thread_updated_at_column(conn: &Connection) -> ForumResult<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let mut has_updated_at = false;
    for col in cols {
        if col.map_err(|e| ForumError::Repo(e.to_string()))? == "updated_at" {
            has_updated_at = true;
            break;
        }
    }
    if !has_updated_at {
        conn.execute(
            "ALTER TABLE threads ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    Ok(())
}

/// Remove all rows (used before a full reindex).
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: threads table is empty.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes all rows.
pub fn clear_all(conn: &Connection) -> ForumResult<()> {
    conn.execute("DELETE FROM evidence", [])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute("DELETE FROM nodes", [])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute("DELETE FROM threads", [])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    Ok(())
}

/// Insert or replace a thread row from a replayed ThreadState.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: thread row is updated with current state.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: writes or updates one row.
pub fn upsert_thread(conn: &Connection, state: &ThreadState) -> ForumResult<()> {
    let tip_sha = state
        .events
        .last()
        .map(|e| e.event_id.as_str())
        .unwrap_or("");
    let updated_at = state
        .events
        .last()
        .map(|e| e.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO threads
         (id, kind, status, title, body, created_at, created_by,
          branch, open_objections, open_actions, has_summary, tip_sha, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            state.id,
            state.kind.to_string(),
            state.status,
            state.title,
            state.body,
            state.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            state.created_by,
            state.branch,
            state.open_objections().len() as i64,
            state.open_actions().len() as i64,
            state.latest_summary().is_some() as i64,
            tip_sha,
            updated_at,
        ],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    Ok(())
}

/// Replace indexed current nodes for a replayed ThreadState.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: node rows for the thread reflect current replayed state.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes and inserts node rows for one thread.
pub fn replace_nodes_for_thread(conn: &Connection, state: &ThreadState) -> ForumResult<()> {
    conn.execute("DELETE FROM nodes WHERE thread_id = ?1", params![state.id])
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO nodes
             (id, thread_id, node_type, status, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    for node in &state.nodes {
        stmt.execute(params![
            node.node_id,
            state.id,
            node.node_type.to_string(),
            node_status(node),
            node.body,
        ])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    Ok(())
}

/// Replace indexed evidence items for a replayed ThreadState.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: evidence rows for the thread reflect current replayed state.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes and inserts evidence rows for one thread.
pub fn replace_evidence_for_thread(conn: &Connection, state: &ThreadState) -> ForumResult<()> {
    conn.execute(
        "DELETE FROM evidence WHERE thread_id = ?1",
        params![state.id],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO evidence
             (id, thread_id, kind, ref_target)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    for ev in &state.evidence_items {
        stmt.execute(params![
            ev.evidence_id,
            state.id,
            ev.kind.to_string(),
            ev.ref_target,
        ])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    Ok(())
}

/// Return all thread rows ordered by ID.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: returns all rows.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: none.
pub fn list_threads(conn: &Connection) -> ForumResult<Vec<ThreadRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, status, title, body, branch, created_at, created_by,
                    open_objections, open_actions, has_summary, tip_sha, coalesce(updated_at, '')
             FROM threads ORDER BY id",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    collect_rows(&mut stmt, [])
}

/// Return a map of thread_id -> tip_sha for all indexed threads.
pub fn thread_tip_shas(
    conn: &Connection,
) -> ForumResult<std::collections::HashMap<String, String>> {
    let mut stmt = conn
        .prepare("SELECT id, tip_sha FROM threads")
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (id, sha) = row.map_err(|e| ForumError::Repo(e.to_string()))?;
        map.insert(id, sha);
    }
    Ok(map)
}

/// Delete a thread and its nodes from the index.
pub fn delete_thread(conn: &Connection, thread_id: &str) -> ForumResult<()> {
    conn.execute(
        "DELETE FROM evidence WHERE thread_id = ?1",
        params![thread_id],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute("DELETE FROM nodes WHERE thread_id = ?1", params![thread_id])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute("DELETE FROM threads WHERE id = ?1", params![thread_id])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    Ok(())
}

/// Find thread IDs that have evidence matching the given kind and ref_target.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: returns matching thread IDs (may be empty).
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: none.
pub fn find_threads_by_evidence_ref(
    conn: &Connection,
    kind: &super::evidence::EvidenceKind,
    ref_target: &str,
) -> ForumResult<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT thread_id FROM evidence
             WHERE kind = ?1 AND ref_target = ?2",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let rows = stmt
        .query_map(params![kind.to_string(), ref_target], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(|e| ForumError::Repo(e.to_string()))?);
    }
    Ok(ids)
}

/// Return thread rows whose indexed thread or current node fields match `query`.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: returns matching rows ordered by ID, with matching current node hits attached.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: none.
pub fn search_threads(conn: &Connection, query: &str) -> ForumResult<Vec<SearchRow>> {
    let pattern = format!("%{query}%");
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, status, title, body, branch, created_at, created_by,
                    open_objections, open_actions, has_summary, tip_sha, coalesce(updated_at, '')
             FROM threads
             WHERE lower(title)  LIKE lower(?1)
                OR lower(id)     LIKE lower(?1)
                OR lower(coalesce(body, '')) LIKE lower(?1)
                OR lower(coalesce(branch, '')) LIKE lower(?1)
                OR lower(kind)   LIKE lower(?1)
                OR lower(status) LIKE lower(?1)
                OR EXISTS (
                    SELECT 1
                    FROM nodes
                    WHERE nodes.thread_id = threads.id
                      AND (
                        lower(nodes.id) LIKE lower(?1)
                        OR lower(nodes.node_type) LIKE lower(?1)
                        OR lower(nodes.status) LIKE lower(?1)
                        OR lower(nodes.body) LIKE lower(?1)
                      )
                )
             ORDER BY id",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let thread_rows = collect_rows(&mut stmt, rusqlite::params![pattern.as_str()])?;
    thread_rows
        .into_iter()
        .map(|thread| {
            let node_hits = search_node_hits(conn, &thread.id, pattern.as_str())?;
            Ok(SearchRow { thread, node_hits })
        })
        .collect()
}

fn collect_rows(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> ForumResult<Vec<ThreadRow>> {
    let iter = stmt
        .query_map(params, |row| {
            Ok(ThreadRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                status: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                branch: row.get(5)?,
                created_at: row.get(6)?,
                created_by: row.get(7)?,
                open_objections: row.get(8)?,
                open_actions: row.get(9)?,
                has_summary: row.get::<_, i64>(10)? != 0,
                tip_sha: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    let mut rows = Vec::new();
    for r in iter {
        rows.push(r.map_err(|e| ForumError::Repo(e.to_string()))?);
    }
    Ok(rows)
}

fn search_node_hits(
    conn: &Connection,
    thread_id: &str,
    pattern: &str,
) -> ForumResult<Vec<NodeSearchHit>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, node_type, status, body
             FROM nodes
             WHERE thread_id = ?1
               AND (
                    lower(id) LIKE lower(?2)
                    OR lower(node_type) LIKE lower(?2)
                    OR lower(status) LIKE lower(?2)
                    OR lower(body) LIKE lower(?2)
               )
             ORDER BY id",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    let iter = stmt
        .query_map(params![thread_id, pattern], |row| {
            Ok(NodeSearchHit {
                node_id: row.get(0)?,
                node_type: row.get(1)?,
                status: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    let mut rows = Vec::new();
    for r in iter {
        rows.push(r.map_err(|e| ForumError::Repo(e.to_string()))?);
    }
    Ok(rows)
}

fn node_status(node: &crate::internal::node::Node) -> &'static str {
    if node.retracted {
        "retracted"
    } else if node.incorporated {
        "incorporated"
    } else if node.resolved {
        "resolved"
    } else {
        "open"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn in_memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    fn make_state(id: &str) -> ThreadState {
        use crate::internal::event::{Event, EventType, ThreadKind};
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: id.into(),
            kind: ThreadKind::Rfc,
            title: "Test RFC".into(),
            body: None,
            branch: None,
            status: "draft".into(),
            created_at: t,
            created_by: "human/alice".into(),
            events: vec![Event {
                event_id: "sha001".into(),
                thread_id: id.into(),
                event_type: EventType::Create,
                created_at: t,
                actor: "human/alice".into(),
                base_rev: None,
                parents: vec![],
                title: Some("Test RFC".into()),
                kind: Some(ThreadKind::Rfc),
                body: None,
                node_type: None,
                target_node_id: None,
                new_state: None,
                approvals: vec![],
                evidence: None,
                link_rel: None,
                branch: None,
                incorporated_node_ids: vec![],
                reply_to: None,
            }],
            nodes: vec![],
            evidence_items: vec![],
            links: vec![],
            body_revision_count: 0,
            incorporated_node_ids: vec![],
        }
    }

    #[test]
    fn upsert_and_list() {
        let conn = in_memory();
        let state = make_state("RFC-0001");
        upsert_thread(&conn, &state).unwrap();

        let rows = list_threads(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "RFC-0001");
        assert_eq!(rows[0].kind, "rfc");
        assert_eq!(rows[0].status, "draft");
        assert_eq!(rows[0].tip_sha, "sha001");
    }

    #[test]
    fn clear_all_empties_table() {
        let conn = in_memory();
        upsert_thread(&conn, &make_state("RFC-0001")).unwrap();
        upsert_thread(&conn, &make_state("RFC-0002")).unwrap();
        clear_all(&conn).unwrap();
        assert!(list_threads(&conn).unwrap().is_empty());
    }

    #[test]
    fn search_matches_title() {
        let conn = in_memory();
        upsert_thread(&conn, &make_state("RFC-0001")).unwrap();
        let results = search_threads(&conn, "Test").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].thread.id, "RFC-0001");
    }

    #[test]
    fn search_no_match_returns_empty() {
        let conn = in_memory();
        upsert_thread(&conn, &make_state("RFC-0001")).unwrap();
        let results = search_threads(&conn, "zzznomatch").unwrap();
        assert!(results.is_empty());
    }

    fn make_state_with_evidence(
        id: &str,
        evidence: Vec<crate::internal::evidence::Evidence>,
    ) -> ThreadState {
        let mut state = make_state(id);
        state.evidence_items = evidence;
        state
    }

    #[test]
    fn replace_evidence_for_thread_inserts_and_replaces() {
        use crate::internal::evidence::{Evidence, EvidenceKind};
        let conn = in_memory();
        let state = make_state_with_evidence(
            "ISSUE-0001",
            vec![Evidence {
                evidence_id: "ev01".into(),
                kind: EvidenceKind::External,
                ref_target: "https://github.com/owner/repo/issues/1".into(),
                locator: None,
            }],
        );
        upsert_thread(&conn, &state).unwrap();
        replace_evidence_for_thread(&conn, &state).unwrap();

        let hits = find_threads_by_evidence_ref(
            &conn,
            &EvidenceKind::External,
            "https://github.com/owner/repo/issues/1",
        )
        .unwrap();
        assert_eq!(hits, vec!["ISSUE-0001"]);

        // Replace with different evidence
        let state2 = make_state_with_evidence(
            "ISSUE-0001",
            vec![Evidence {
                evidence_id: "ev02".into(),
                kind: EvidenceKind::External,
                ref_target: "https://github.com/owner/repo/issues/2".into(),
                locator: None,
            }],
        );
        replace_evidence_for_thread(&conn, &state2).unwrap();

        // Old evidence is gone
        let old = find_threads_by_evidence_ref(
            &conn,
            &EvidenceKind::External,
            "https://github.com/owner/repo/issues/1",
        )
        .unwrap();
        assert!(old.is_empty());

        // New evidence is present
        let new = find_threads_by_evidence_ref(
            &conn,
            &EvidenceKind::External,
            "https://github.com/owner/repo/issues/2",
        )
        .unwrap();
        assert_eq!(new, vec!["ISSUE-0001"]);
    }

    #[test]
    fn find_threads_by_evidence_ref_filters_by_kind() {
        use crate::internal::evidence::{Evidence, EvidenceKind};
        let conn = in_memory();
        let state = make_state_with_evidence(
            "ISSUE-0001",
            vec![Evidence {
                evidence_id: "ev01".into(),
                kind: EvidenceKind::Commit,
                ref_target: "abc123".into(),
                locator: None,
            }],
        );
        upsert_thread(&conn, &state).unwrap();
        replace_evidence_for_thread(&conn, &state).unwrap();

        // Wrong kind returns empty
        let hits = find_threads_by_evidence_ref(&conn, &EvidenceKind::External, "abc123").unwrap();
        assert!(hits.is_empty());

        // Right kind returns match
        let hits = find_threads_by_evidence_ref(&conn, &EvidenceKind::Commit, "abc123").unwrap();
        assert_eq!(hits, vec!["ISSUE-0001"]);
    }

    #[test]
    fn clear_all_clears_evidence() {
        use crate::internal::evidence::{Evidence, EvidenceKind};
        let conn = in_memory();
        let state = make_state_with_evidence(
            "ISSUE-0001",
            vec![Evidence {
                evidence_id: "ev01".into(),
                kind: EvidenceKind::External,
                ref_target: "https://example.com".into(),
                locator: None,
            }],
        );
        upsert_thread(&conn, &state).unwrap();
        replace_evidence_for_thread(&conn, &state).unwrap();
        clear_all(&conn).unwrap();

        let hits =
            find_threads_by_evidence_ref(&conn, &EvidenceKind::External, "https://example.com")
                .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn delete_thread_clears_evidence() {
        use crate::internal::evidence::{Evidence, EvidenceKind};
        let conn = in_memory();
        let state = make_state_with_evidence(
            "ISSUE-0001",
            vec![Evidence {
                evidence_id: "ev01".into(),
                kind: EvidenceKind::External,
                ref_target: "https://example.com".into(),
                locator: None,
            }],
        );
        upsert_thread(&conn, &state).unwrap();
        replace_evidence_for_thread(&conn, &state).unwrap();
        delete_thread(&conn, "ISSUE-0001").unwrap();

        let hits =
            find_threads_by_evidence_ref(&conn, &EvidenceKind::External, "https://example.com")
                .unwrap();
        assert!(hits.is_empty());
    }
}
