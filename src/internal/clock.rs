use chrono::{DateTime, Utc};

/// Swappable clock for deterministic testing.
///
/// Production code uses [`SystemClock`]. Tests inject [`FixedClock`] or
/// [`StepClock`] via the same trait.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Real wall-clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Always returns the same instant — useful for snapshot-stable tests.
pub struct FixedClock {
    pub instant: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.instant
    }
}

/// Advances by a fixed step on each call — useful for ordering tests.
pub struct StepClock {
    start: DateTime<Utc>,
    step: chrono::Duration,
    counter: std::sync::atomic::AtomicU64,
}

impl StepClock {
    pub fn new(start: DateTime<Utc>, step: chrono::Duration) -> Self {
        Self {
            start,
            step,
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl Clock for StepClock {
    fn now(&self) -> DateTime<Utc> {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.start + self.step * n as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn fixed_clock_returns_same_instant() {
        let clock = FixedClock {
            instant: fixed_time(),
        };
        assert_eq!(clock.now(), clock.now());
    }

    #[test]
    fn step_clock_advances() {
        let clock = StepClock::new(fixed_time(), chrono::Duration::seconds(10));
        let t0 = clock.now();
        let t1 = clock.now();
        let t2 = clock.now();
        assert_eq!((t1 - t0).num_seconds(), 10);
        assert_eq!((t2 - t1).num_seconds(), 10);
    }
}
