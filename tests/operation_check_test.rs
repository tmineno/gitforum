//! Operation-check integration tests.
//!
//! The bulk of v2 operation-check coverage is replaced by SPEC-3.0
//! §3.3 internal tests in `src/internal/operation_check.rs`. v2 paths
//! covered here (legacy lifecycle/tag scoping, kind-keyed creation
//! rules, etc.) are removed by task `913c4s9v`. Smoke-level integration is
//! covered by `state_change_test.rs` and `verify_test.rs`.
