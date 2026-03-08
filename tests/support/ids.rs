// Re-export ID generator types from the main crate for test convenience.
//
// Tests that need deterministic IDs should use:
//   SequentialIdGenerator — produces "prefix-0001", "prefix-0002", …
//
// These live in `src/internal/id.rs` and are imported here so that
// integration tests have a single import path.
//
// Example:
//   use support::ids; // then use git_forum::internal::id::SequentialIdGenerator
