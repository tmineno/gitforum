// Re-export clock types from the main crate for test convenience.
//
// Tests that need a deterministic clock should use:
//   FixedClock  — always returns the same instant
//   StepClock   — advances by a fixed delta on each call
//
// These live in `src/internal/clock.rs` and are imported here so that
// integration tests have a single import path.
//
// Example:
//   use support::clock; // then use git_forum::internal::clock::FixedClock
