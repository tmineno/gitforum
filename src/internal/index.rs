use std::path::Path;

use rusqlite::{params, Connection};

use super::error::{ForumError, ForumResult};
use super::thread::ThreadState;

/// A denormalized thread row cached in the local SQLite index.
#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    pub kind: String,
    /// Phase 3: lifecycle is now a real indexed column, not a kind-derived
    /// inference at read time.
    pub lifecycle: String,
    /// `true` iff a `facet_set` event in the chain explicitly set
    /// `lifecycle` (matches `ThreadState::lifecycle_explicit`).
    pub lifecycle_explicit: bool,
    /// Phase 3: replayed tag set, joined from `thread_tags`.
    pub tags: Vec<String>,
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

/// Phase 3 (Finding 2): v2 schema — adds `lifecycle` /
/// `lifecycle_explicit` columns to `threads` and a many-to-many
/// `thread_tags` table. The legacy `kind` column stays for
/// storage-compat reads (per ADR-002), but lifecycle/tags are now the
/// canonical filter axes and search query (`lifecycle:` / `tag:`)
/// targets.
///
/// The index is a derived cache of git refs and can be safely wiped, so
/// `user_version` < 2 (fresh DB or v1 from before Phase 3) drops every
/// table and recreates. The next `reindex` rebuilds it from refs.
const SCHEMA_V2: &str = "CREATE TABLE IF NOT EXISTS threads (
            id                  TEXT PRIMARY KEY,
            kind                TEXT NOT NULL,
            lifecycle           TEXT NOT NULL DEFAULT 'execution',
            lifecycle_explicit  INTEGER NOT NULL DEFAULT 0,
            status              TEXT NOT NULL,
            title               TEXT NOT NULL,
            body                TEXT,
            branch              TEXT,
            created_at          TEXT NOT NULL,
            created_by          TEXT NOT NULL,
            open_objections     INTEGER NOT NULL DEFAULT 0,
            open_actions        INTEGER NOT NULL DEFAULT 0,
            has_summary         INTEGER NOT NULL DEFAULT 0,
            tip_sha             TEXT NOT NULL,
            updated_at          TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_threads_lifecycle ON threads(lifecycle);
        CREATE TABLE IF NOT EXISTS thread_tags (
            thread_id TEXT NOT NULL,
            tag       TEXT NOT NULL,
            PRIMARY KEY (thread_id, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_thread_tags_tag ON thread_tags(tag);
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
        CREATE INDEX IF NOT EXISTS idx_evidence_ref ON evidence(ref_target);
        CREATE TABLE IF NOT EXISTS links (
            from_thread_id  TEXT NOT NULL,
            to_thread_id    TEXT NOT NULL,
            rel             TEXT NOT NULL,
            PRIMARY KEY (from_thread_id, to_thread_id, rel)
        );
        CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_thread_id, rel);
        PRAGMA user_version = 2;";

fn ensure_schema(conn: &Connection) -> ForumResult<()> {
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    if version < 2 {
        // Pre-Phase-3 schema (fresh DB, 1.x leftover, or v1 from
        // before lifecycle/tags became indexable columns): drop every
        // table and recreate. Safe because `reindex` rebuilds the
        // entire index from git refs.
        conn.execute_batch(
            "DROP TABLE IF EXISTS thread_tags;
             DROP TABLE IF EXISTS threads;
             DROP TABLE IF EXISTS nodes;
             DROP TABLE IF EXISTS evidence;
             DROP TABLE IF EXISTS links;",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    conn.execute_batch(SCHEMA_V2)
        .map_err(|e| ForumError::Repo(e.to_string()))?;
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
    conn.execute("DELETE FROM links", [])
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute("DELETE FROM thread_tags", [])
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
         (id, kind, lifecycle, lifecycle_explicit, status, title, body,
          created_at, created_by, branch,
          open_objections, open_actions, has_summary, tip_sha, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            state.id,
            state.kind.to_string(),
            state.lifecycle.as_str(),
            state.lifecycle_explicit as i64,
            state.status.as_str(),
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

/// Replace indexed tag rows for a replayed ThreadState.
///
/// Preconditions: `conn` is open with v2 schema applied; `state.id` row exists.
/// Postconditions: `thread_tags` rows for the thread reflect `state.tags` exactly.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes and inserts tag rows for one thread.
pub fn replace_tags_for_thread(conn: &Connection, state: &ThreadState) -> ForumResult<()> {
    conn.execute(
        "DELETE FROM thread_tags WHERE thread_id = ?1",
        params![state.id],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    if state.tags.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare("INSERT OR REPLACE INTO thread_tags (thread_id, tag) VALUES (?1, ?2)")
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    for tag in &state.tags {
        stmt.execute(params![state.id, tag])
            .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
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
            node.status(),
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

/// Replace indexed link rows for a replayed ThreadState.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: link rows where from_thread_id == state.id reflect the thread's current
///   forward link set (state.links). Each link's `to_thread_id` is canonicalized to the
///   bare-token form when the legacy form is found via the deterministic migrate mapping
///   (per Track C migrate, SPEC-2.0 §10.1) — so that incoming-link queries against the
///   canonical thread ID match links that pre-date the migration.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: deletes and inserts link rows for one thread.
pub fn replace_links_for_thread(conn: &Connection, state: &ThreadState) -> ForumResult<()> {
    conn.execute(
        "DELETE FROM links WHERE from_thread_id = ?1",
        params![state.id],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;

    let known_ids: std::collections::HashSet<String> = thread_tip_shas(conn)?.into_keys().collect();

    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO links
             (from_thread_id, to_thread_id, rel)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    for link in &state.links {
        let canonical = canonicalize_link_target(&link.target_thread_id, &known_ids);
        stmt.execute(params![state.id, canonical, link.rel,])
            .map_err(|e| ForumError::Repo(e.to_string()))?;
    }
    Ok(())
}

/// Map a link target ID to its canonical thread ID when possible.
///
/// Link events serialize the target ID in whatever form the author typed at
/// commit time — frequently the legacy `KIND-XXXXXXXX` form for events that
/// pre-date the Track C migration. The reverse-link index is queried against
/// the canonical bare-token ID, so we normalize at write time using the
/// migrator's deterministic mapping (`migrate::bare_token_for`) — but only
/// when the resulting canonical token is observably a real thread, so we
/// don't rewrite targets that point at threads outside this repo.
fn canonicalize_link_target(target: &str, known_ids: &std::collections::HashSet<String>) -> String {
    if known_ids.contains(target) {
        return target.to_string();
    }
    let canonical = super::migrate::bare_token_for(target);
    if canonical != target && known_ids.contains(&canonical) {
        canonical
    } else {
        target.to_string()
    }
}

/// An incoming link row: a thread that links to the queried thread.
#[derive(Debug, Clone)]
pub struct IncomingLink {
    pub from_thread_id: String,
    pub rel: String,
}

/// Return threads that link to `to_thread_id`, optionally filtered by relation.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: returns matching incoming-link rows ordered by from_thread_id.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: none. This is the primary read for `show --tree` (one indexed lookup).
pub fn find_incoming_links(
    conn: &Connection,
    to_thread_id: &str,
    rel: Option<&str>,
) -> ForumResult<Vec<IncomingLink>> {
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(IncomingLink {
            from_thread_id: row.get(0)?,
            rel: row.get(1)?,
        })
    };
    let mut out = Vec::new();
    if let Some(r) = rel {
        let mut stmt = conn
            .prepare(
                "SELECT from_thread_id, rel FROM links
                 WHERE to_thread_id = ?1 AND rel = ?2
                 ORDER BY from_thread_id",
            )
            .map_err(|e| ForumError::Repo(e.to_string()))?;
        let iter = stmt
            .query_map(params![to_thread_id, r], map_row)
            .map_err(|e| ForumError::Repo(e.to_string()))?;
        for row in iter {
            out.push(row.map_err(|e| ForumError::Repo(e.to_string()))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT from_thread_id, rel FROM links
                 WHERE to_thread_id = ?1
                 ORDER BY from_thread_id, rel",
            )
            .map_err(|e| ForumError::Repo(e.to_string()))?;
        let iter = stmt
            .query_map(params![to_thread_id], map_row)
            .map_err(|e| ForumError::Repo(e.to_string()))?;
        for row in iter {
            out.push(row.map_err(|e| ForumError::Repo(e.to_string()))?);
        }
    }
    Ok(out)
}

/// Return one indexed thread row by ID, if present.
///
/// Preconditions: `conn` is open with schema applied.
/// Postconditions: returns Some(row) if a thread with `id` exists; None otherwise.
/// Failure modes: ForumError::Repo on SQL error.
/// Side effects: none.
pub fn get_thread(conn: &Connection, id: &str) -> ForumResult<Option<ThreadRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, lifecycle, lifecycle_explicit,
                    status, title, body, branch, created_at, created_by,
                    open_objections, open_actions, has_summary, tip_sha, coalesce(updated_at, '')
             FROM threads WHERE id = ?1",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let mut rows = collect_rows(conn, &mut stmt, params![id])?;
    Ok(rows.pop())
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
            "SELECT id, kind, lifecycle, lifecycle_explicit,
                    status, title, body, branch, created_at, created_by,
                    open_objections, open_actions, has_summary, tip_sha, coalesce(updated_at, '')
             FROM threads ORDER BY id",
        )
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    collect_rows(conn, &mut stmt, [])
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
    conn.execute(
        "DELETE FROM links WHERE from_thread_id = ?1",
        params![thread_id],
    )
    .map_err(|e| ForumError::Repo(e.to_string()))?;
    conn.execute(
        "DELETE FROM thread_tags WHERE thread_id = ?1",
        params![thread_id],
    )
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
///
/// Phase 3: query is split on whitespace and tokens are routed by prefix:
/// `lifecycle:VALUE` -> exact match against the `lifecycle` column,
/// `tag:VALUE` -> exact match against `thread_tags.tag`,
/// any other token -> substring search across the prior free-text columns.
/// All tokens are AND-ed; a query of one bare word reproduces the
/// previous substring behaviour.
pub fn search_threads(conn: &Connection, query: &str) -> ForumResult<Vec<SearchRow>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut bind: Vec<String> = Vec::new();
    for token in query.split_whitespace() {
        if token.eq_ignore_ascii_case("AND") {
            // Boolean noise from translators (`kind:rfc -> "lifecycle:proposal AND tag:cross-cutting"`).
            // Tokens are already AND-ed implicitly, so drop the keyword.
            continue;
        }
        if let Some(value) = token.strip_prefix("lifecycle:") {
            clauses.push(format!("lower(lifecycle) = lower(?{})", bind.len() + 1));
            bind.push(value.to_string());
        } else if let Some(value) = token.strip_prefix("tag:") {
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM thread_tags
                         WHERE thread_tags.thread_id = threads.id
                           AND lower(thread_tags.tag) = lower(?{}))",
                bind.len() + 1
            ));
            bind.push(value.to_string());
        } else {
            let placeholder = format!("?{}", bind.len() + 1);
            clauses.push(format!(
                "(lower(title) LIKE lower({p})
                 OR lower(id) LIKE lower({p})
                 OR lower(coalesce(body, '')) LIKE lower({p})
                 OR lower(coalesce(branch, '')) LIKE lower({p})
                 OR lower(kind) LIKE lower({p})
                 OR lower(lifecycle) LIKE lower({p})
                 OR lower(status) LIKE lower({p})
                 OR EXISTS (
                     SELECT 1 FROM thread_tags
                     WHERE thread_tags.thread_id = threads.id
                       AND lower(thread_tags.tag) LIKE lower({p})
                 )
                 OR EXISTS (
                     SELECT 1 FROM nodes
                     WHERE nodes.thread_id = threads.id
                       AND (
                         lower(nodes.id) LIKE lower({p})
                         OR lower(nodes.node_type) LIKE lower({p})
                         OR lower(nodes.status) LIKE lower({p})
                         OR lower(nodes.body) LIKE lower({p})
                       )
                 ))",
                p = placeholder
            ));
            bind.push(format!("%{token}%"));
        }
    }
    if clauses.is_empty() {
        return Ok(Vec::new());
    }
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT id, kind, lifecycle, lifecycle_explicit,
                status, title, body, branch, created_at, created_by,
                open_objections, open_actions, has_summary, tip_sha, coalesce(updated_at, '')
         FROM threads
         WHERE {where_sql}
         ORDER BY id"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    let bind_refs: Vec<&dyn rusqlite::ToSql> =
        bind.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let thread_rows = collect_rows(conn, &mut stmt, bind_refs.as_slice())?;
    // Node hits use the first free-text token's pattern when present, so
    // a bare-word query keeps producing per-node matches. Pure facet
    // queries (`lifecycle:proposal`) get no node hits — by design.
    let node_pattern = bind
        .iter()
        .find(|s| s.starts_with('%') && s.ends_with('%'))
        .cloned();
    thread_rows
        .into_iter()
        .map(|thread| {
            let node_hits = match &node_pattern {
                Some(p) => search_node_hits(conn, &thread.id, p)?,
                None => Vec::new(),
            };
            Ok(SearchRow { thread, node_hits })
        })
        .collect()
}

fn collect_rows(
    conn: &Connection,
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> ForumResult<Vec<ThreadRow>> {
    let iter = stmt
        .query_map(params, |row| {
            Ok(ThreadRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                lifecycle: row.get(2)?,
                lifecycle_explicit: row.get::<_, i64>(3)? != 0,
                tags: Vec::new(), // filled below in a separate pass
                status: row.get(4)?,
                title: row.get(5)?,
                body: row.get(6)?,
                branch: row.get(7)?,
                created_at: row.get(8)?,
                created_by: row.get(9)?,
                open_objections: row.get(10)?,
                open_actions: row.get(11)?,
                has_summary: row.get::<_, i64>(12)? != 0,
                tip_sha: row.get(13)?,
                updated_at: row.get(14)?,
            })
        })
        .map_err(|e| ForumError::Repo(e.to_string()))?;

    let mut rows = Vec::new();
    for r in iter {
        rows.push(r.map_err(|e| ForumError::Repo(e.to_string()))?);
    }
    // Phase 3: hydrate the tags list from `thread_tags` so callers see the
    // full ThreadRow without an extra join in every projection. Only one
    // extra query per row; row counts here are bounded by the caller's
    // filter (single get_thread, list_threads <= total threads).
    let mut tag_stmt = conn
        .prepare("SELECT tag FROM thread_tags WHERE thread_id = ?1 ORDER BY tag")
        .map_err(|e| ForumError::Repo(e.to_string()))?;
    for row in &mut rows {
        let tags = tag_stmt
            .query_map(params![row.id], |r| r.get::<_, String>(0))
            .map_err(|e| ForumError::Repo(e.to_string()))?;
        row.tags = tags
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| ForumError::Repo(e.to_string()))?;
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
        use crate::internal::event::{Event, EventType, Lifecycle, ThreadKind, ThreadStatus};
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: id.into(),
            kind: ThreadKind::Rfc,
            // Phase 3: keep lifecycle aligned with kind in fixtures.
            lifecycle: Lifecycle::Proposal,
            title: "Test RFC".into(),
            status: ThreadStatus::Draft,
            created_at: t,
            created_by: "human/alice".into(),
            events: vec![Event {
                event_id: "sha001".into(),
                thread_id: id.into(),
                event_type: EventType::Create,
                created_at: t,
                actor: "human/alice".into(),
                title: Some("Test RFC".into()),
                kind: Some(ThreadKind::Rfc),
                ..Event::default()
            }],
            ..ThreadState::default()
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
        // Phase 3: lifecycle is now an indexed column, not a kind-derived guess.
        assert_eq!(rows[0].lifecycle, "proposal");
        assert!(!rows[0].lifecycle_explicit);
        assert!(rows[0].tags.is_empty());
        assert_eq!(rows[0].status, "draft");
        assert_eq!(rows[0].tip_sha, "sha001");
    }

    #[test]
    fn upsert_persists_lifecycle_explicit_and_tags() {
        use crate::internal::event::Lifecycle;
        let conn = in_memory();
        let mut state = make_state("RFC-0001");
        state.lifecycle = Lifecycle::Execution; // override + simulate facet_set write
        state.lifecycle_explicit = true;
        state.tags = vec!["bug".into(), "ux".into()];
        upsert_thread(&conn, &state).unwrap();
        replace_tags_for_thread(&conn, &state).unwrap();

        let rows = list_threads(&conn).unwrap();
        assert_eq!(rows[0].lifecycle, "execution");
        assert!(rows[0].lifecycle_explicit);
        // Phase 3: tags come back sorted (`ORDER BY tag` in collect_rows).
        assert_eq!(rows[0].tags, vec!["bug".to_string(), "ux".to_string()]);
    }

    #[test]
    fn replace_tags_replaces_full_set() {
        let conn = in_memory();
        let mut state = make_state("RFC-0001");
        state.tags = vec!["bug".into(), "ux".into()];
        upsert_thread(&conn, &state).unwrap();
        replace_tags_for_thread(&conn, &state).unwrap();

        // Subsequent replace with a different set wins entirely.
        state.tags = vec!["task".into()];
        replace_tags_for_thread(&conn, &state).unwrap();

        let rows = list_threads(&conn).unwrap();
        assert_eq!(rows[0].tags, vec!["task".to_string()]);
    }

    #[test]
    fn search_matches_lifecycle_column() {
        use crate::internal::event::Lifecycle;
        let conn = in_memory();
        let mut s1 = make_state("RFC-0001");
        s1.lifecycle = Lifecycle::Proposal;
        let mut s2 = make_state("ASK-0001");
        s2.lifecycle = Lifecycle::Execution;
        s2.title = "Other".into();
        upsert_thread(&conn, &s1).unwrap();
        upsert_thread(&conn, &s2).unwrap();

        let proposals = search_threads(&conn, "proposal").unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].thread.id, "RFC-0001");

        let executions = search_threads(&conn, "execution").unwrap();
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].thread.id, "ASK-0001");
    }

    #[test]
    fn search_lifecycle_prefix_token_filters_by_column() {
        // Phase 3: `lifecycle:proposal` is an EXACT column filter, not a
        // free-text substring. A thread whose body merely mentions the
        // word "proposal" must NOT match this token.
        use crate::internal::event::Lifecycle;
        let conn = in_memory();
        let mut s1 = make_state("RFC-0001");
        s1.lifecycle = Lifecycle::Proposal;
        let mut s2 = make_state("ASK-0001");
        s2.title = "discussion of proposal text".into();
        s2.body = Some("body mentions proposal".into());
        s2.lifecycle = Lifecycle::Execution;
        upsert_thread(&conn, &s1).unwrap();
        upsert_thread(&conn, &s2).unwrap();

        let hits = search_threads(&conn, "lifecycle:proposal").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.id, "RFC-0001");
    }

    #[test]
    fn search_tag_prefix_token_uses_thread_tags_join() {
        // Bare-word "bug" matches anywhere; `tag:bug` only matches the
        // thread_tags row.
        let conn = in_memory();
        let mut s1 = make_state("RFC-0001");
        s1.tags = vec!["cross-cutting".into()];
        s1.body = Some("incidentally mentions bug".into());
        let mut s2 = make_state("ASK-0001");
        s2.title = "Other".into();
        s2.tags = vec!["bug".into()];
        upsert_thread(&conn, &s1).unwrap();
        replace_tags_for_thread(&conn, &s1).unwrap();
        upsert_thread(&conn, &s2).unwrap();
        replace_tags_for_thread(&conn, &s2).unwrap();

        let hits = search_threads(&conn, "tag:bug").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.id, "ASK-0001");
    }

    #[test]
    fn search_combines_prefix_and_freetext_with_and() {
        // Combined: `lifecycle:execution Other` -> the thread must match
        // both. Verifies the conjunction wiring of the new query parser.
        use crate::internal::event::Lifecycle;
        let conn = in_memory();
        let mut s1 = make_state("RFC-0001");
        s1.lifecycle = Lifecycle::Proposal;
        s1.title = "Other".into();
        let mut s2 = make_state("ASK-0001");
        s2.lifecycle = Lifecycle::Execution;
        s2.title = "Other".into();
        upsert_thread(&conn, &s1).unwrap();
        upsert_thread(&conn, &s2).unwrap();

        let hits = search_threads(&conn, "lifecycle:execution Other").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.id, "ASK-0001");
    }

    #[test]
    fn search_matches_thread_tag() {
        let conn = in_memory();
        let mut s1 = make_state("RFC-0001");
        s1.tags = vec!["cross-cutting".into()];
        let mut s2 = make_state("ASK-0001");
        s2.title = "Other".into();
        s2.tags = vec!["bug".into()];
        upsert_thread(&conn, &s1).unwrap();
        replace_tags_for_thread(&conn, &s1).unwrap();
        upsert_thread(&conn, &s2).unwrap();
        replace_tags_for_thread(&conn, &s2).unwrap();

        let bugs = search_threads(&conn, "bug").unwrap();
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].thread.id, "ASK-0001");
    }

    #[test]
    fn delete_thread_removes_tags() {
        let conn = in_memory();
        let mut state = make_state("RFC-0001");
        state.tags = vec!["bug".into()];
        upsert_thread(&conn, &state).unwrap();
        replace_tags_for_thread(&conn, &state).unwrap();
        assert_eq!(
            list_threads(&conn).unwrap()[0].tags,
            vec!["bug".to_string()]
        );

        delete_thread(&conn, "RFC-0001").unwrap();
        let leftover_tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM thread_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover_tags, 0);
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

    fn make_state_with_links(
        id: &str,
        links: Vec<crate::internal::thread::ThreadLink>,
    ) -> ThreadState {
        let mut state = make_state(id);
        state.links = links;
        state
    }

    #[test]
    fn replace_links_and_query_incoming() {
        use crate::internal::thread::ThreadLink;
        let conn = in_memory();

        // Two threads link to RFC-PARENT with `implements`; one links with `relates-to`.
        let child1 = make_state_with_links(
            "TASK-CHILD1",
            vec![ThreadLink {
                target_thread_id: "RFC-PARENT".into(),
                rel: "implements".into(),
            }],
        );
        let child2 = make_state_with_links(
            "TASK-CHILD2",
            vec![ThreadLink {
                target_thread_id: "RFC-PARENT".into(),
                rel: "implements".into(),
            }],
        );
        let sibling = make_state_with_links(
            "DEC-SIBLING",
            vec![ThreadLink {
                target_thread_id: "RFC-PARENT".into(),
                rel: "relates-to".into(),
            }],
        );
        for s in [&child1, &child2, &sibling] {
            upsert_thread(&conn, s).unwrap();
            replace_links_for_thread(&conn, s).unwrap();
        }

        let implements = find_incoming_links(&conn, "RFC-PARENT", Some("implements")).unwrap();
        assert_eq!(implements.len(), 2);
        let from_ids: Vec<&str> = implements
            .iter()
            .map(|l| l.from_thread_id.as_str())
            .collect();
        assert_eq!(from_ids, vec!["TASK-CHILD1", "TASK-CHILD2"]);

        let all = find_incoming_links(&conn, "RFC-PARENT", None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn replace_links_overwrites_old_entries() {
        use crate::internal::thread::ThreadLink;
        let conn = in_memory();
        let state1 = make_state_with_links(
            "TASK-A",
            vec![ThreadLink {
                target_thread_id: "RFC-OLD".into(),
                rel: "implements".into(),
            }],
        );
        upsert_thread(&conn, &state1).unwrap();
        replace_links_for_thread(&conn, &state1).unwrap();

        // Re-replay with a different link target — old row must be removed.
        let state2 = make_state_with_links(
            "TASK-A",
            vec![ThreadLink {
                target_thread_id: "RFC-NEW".into(),
                rel: "implements".into(),
            }],
        );
        replace_links_for_thread(&conn, &state2).unwrap();

        assert!(find_incoming_links(&conn, "RFC-OLD", None)
            .unwrap()
            .is_empty());
        let new_hits = find_incoming_links(&conn, "RFC-NEW", Some("implements")).unwrap();
        assert_eq!(new_hits.len(), 1);
        assert_eq!(new_hits[0].from_thread_id, "TASK-A");
    }

    #[test]
    fn delete_thread_clears_links() {
        use crate::internal::thread::ThreadLink;
        let conn = in_memory();
        let state = make_state_with_links(
            "TASK-DEL",
            vec![ThreadLink {
                target_thread_id: "RFC-X".into(),
                rel: "implements".into(),
            }],
        );
        upsert_thread(&conn, &state).unwrap();
        replace_links_for_thread(&conn, &state).unwrap();
        delete_thread(&conn, "TASK-DEL").unwrap();
        assert!(find_incoming_links(&conn, "RFC-X", None)
            .unwrap()
            .is_empty());
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
