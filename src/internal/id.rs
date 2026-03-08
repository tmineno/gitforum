/// Swappable ID generator for deterministic testing.
///
/// Production code uses [`UlidGenerator`]. Tests inject [`SequentialIdGenerator`]
/// for predictable, diff-friendly output.
pub trait IdGenerator: Send + Sync {
    /// Generate a new unique internal ID (opaque string).
    fn next_id(&self) -> String;
}

/// Generates ULID-style IDs (monotonic, sortable, random).
pub struct UlidGenerator;

impl IdGenerator for UlidGenerator {
    fn next_id(&self) -> String {
        // Minimal ULID-like: timestamp-ms + random suffix.
        // A proper ULID crate can replace this later.
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let rand: u64 = rand_u64();
        format!("{ts:013x}-{rand:016x}")
    }
}

/// Simple counter — produces `"test-0001"`, `"test-0002"`, … for tests.
pub struct SequentialIdGenerator {
    prefix: String,
    counter: std::sync::atomic::AtomicU64,
}

impl SequentialIdGenerator {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl IdGenerator for SequentialIdGenerator {
    fn next_id(&self) -> String {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("{}-{n:04}", self.prefix)
    }
}

/// Minimal random u64 without pulling in the `rand` crate.
fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_id_generator() {
        let gen = SequentialIdGenerator::new("test");
        assert_eq!(gen.next_id(), "test-0001");
        assert_eq!(gen.next_id(), "test-0002");
        assert_eq!(gen.next_id(), "test-0003");
    }

    #[test]
    fn ulid_generator_produces_unique_ids() {
        let gen = UlidGenerator;
        let a = gen.next_id();
        let b = gen.next_id();
        assert_ne!(a, b);
    }
}
