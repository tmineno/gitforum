//! LRU thread replay cache for the TUI (RFC-0017 Phase 1).
//!
//! Caches the result of `replay_thread()` keyed by `(thread_id, tip_sha)`.
//! On cache hit, the TUI skips the git replay entirely.

use std::collections::HashMap;

use crate::internal::thread::ThreadState;

const MAX_ENTRIES: usize = 16;

struct CacheEntry {
    tip_sha: String,
    state: ThreadState,
}

/// LRU cache for replayed thread states.
pub(crate) struct ReplayCache {
    entries: HashMap<String, CacheEntry>,
    /// LRU order: most recently used at the end.
    order: Vec<String>,
}

impl ReplayCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Look up a cached thread state. Returns `Some` only if both thread_id
    /// and tip_sha match. Promotes the entry to most-recently-used on hit.
    pub fn get(&mut self, thread_id: &str, tip_sha: &str) -> Option<&ThreadState> {
        let entry = self.entries.get(thread_id)?;
        if entry.tip_sha != tip_sha {
            return None;
        }
        // Promote to MRU
        if let Some(pos) = self.order.iter().position(|id| id == thread_id) {
            self.order.remove(pos);
            self.order.push(thread_id.to_string());
        }
        Some(&self.entries[thread_id].state)
    }

    /// Insert or update a cached thread state. Evicts the LRU entry if at capacity.
    pub fn insert(&mut self, thread_id: String, tip_sha: String, state: ThreadState) {
        // If already present, update in place and promote
        if self.entries.contains_key(&thread_id) {
            self.entries
                .insert(thread_id.clone(), CacheEntry { tip_sha, state });
            if let Some(pos) = self.order.iter().position(|id| id == &thread_id) {
                self.order.remove(pos);
            }
            self.order.push(thread_id);
            return;
        }

        // Evict LRU if at capacity
        if self.entries.len() >= MAX_ENTRIES {
            if let Some(lru_id) = self.order.first().cloned() {
                self.entries.remove(&lru_id);
                self.order.remove(0);
            }
        }

        self.entries
            .insert(thread_id.clone(), CacheEntry { tip_sha, state });
        self.order.push(thread_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_state(id: &str) -> ThreadState {
        ThreadState {
            id: id.to_string(),
            kind: crate::internal::event::ThreadKind::Rfc,
            title: format!("Thread {id}"),
            body: None,
            branch: None,
            status: "draft".to_string(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            created_by: "human/test".to_string(),
            events: vec![],
            nodes: vec![],
            evidence_items: vec![],
            links: vec![],
            body_revision_count: 0,
            incorporated_node_ids: vec![],
        }
    }

    #[test]
    fn cache_hit_returns_state() {
        let mut cache = ReplayCache::new();
        cache.insert("RFC-0001".into(), "sha1".into(), make_state("RFC-0001"));
        assert!(cache.get("RFC-0001", "sha1").is_some());
    }

    #[test]
    fn cache_miss_on_wrong_sha() {
        let mut cache = ReplayCache::new();
        cache.insert("RFC-0001".into(), "sha1".into(), make_state("RFC-0001"));
        assert!(cache.get("RFC-0001", "sha2").is_none());
    }

    #[test]
    fn cache_miss_on_unknown_id() {
        let mut cache = ReplayCache::new();
        cache.insert("RFC-0001".into(), "sha1".into(), make_state("RFC-0001"));
        assert!(cache.get("RFC-0002", "sha1").is_none());
    }

    #[test]
    fn evicts_lru_at_capacity() {
        let mut cache = ReplayCache::new();
        for i in 0..MAX_ENTRIES {
            let id = format!("RFC-{i:04}");
            cache.insert(id.clone(), format!("sha{i}"), make_state(&id));
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);

        // Insert one more — should evict RFC-0000
        cache.insert("RFC-NEW".into(), "sha_new".into(), make_state("RFC-NEW"));
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get("RFC-0000", "sha0").is_none());
        assert!(cache.get("RFC-NEW", "sha_new").is_some());
    }

    #[test]
    fn get_promotes_to_mru() {
        let mut cache = ReplayCache::new();
        for i in 0..MAX_ENTRIES {
            let id = format!("RFC-{i:04}");
            cache.insert(id.clone(), format!("sha{i}"), make_state(&id));
        }

        // Access RFC-0000 to promote it
        assert!(cache.get("RFC-0000", "sha0").is_some());

        // Insert one more — should evict RFC-0001 (new LRU), not RFC-0000
        cache.insert("RFC-NEW".into(), "sha_new".into(), make_state("RFC-NEW"));
        assert!(cache.get("RFC-0000", "sha0").is_some());
        assert!(cache.get("RFC-0001", "sha1").is_none());
    }

    #[test]
    fn update_existing_entry() {
        let mut cache = ReplayCache::new();
        cache.insert("RFC-0001".into(), "sha1".into(), make_state("RFC-0001"));
        cache.insert("RFC-0001".into(), "sha2".into(), make_state("RFC-0001"));
        assert!(cache.get("RFC-0001", "sha1").is_none());
        assert!(cache.get("RFC-0001", "sha2").is_some());
        assert_eq!(cache.entries.len(), 1);
    }
}
