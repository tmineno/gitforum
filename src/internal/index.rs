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
        );",
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    ensure_thread_branch_column(conn)
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

/// Remove all rows (used before a full reindex).
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: threads table is empty.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes all rows.
pub fn clear_all(conn: &Connection) -> ForumResult<()> {
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
    conn.execute(
        "INSERT OR REPLACE INTO threads
         (id, kind, status, title, body, created_at, created_by,
          branch, open_objections, open_actions, has_summary, tip_sha)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
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
                    open_objections, open_actions, has_summary, tip_sha
             FROM threads ORDER BY id",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    collect_rows(&mut stmt, [])
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
                    open_objections, open_actions, has_summary, tip_sha
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
                run_label: None,
                branch: None,
            }],
            nodes: vec![],
            evidence_items: vec![],
            links: vec![],
            run_labels: vec![],
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
}
