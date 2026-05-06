//! Publish pipeline for `git forum push` (RFC `fls856j3`).
//!
//! - [`exclusion`] — pure transformation that filters a public
//!   thread's `links.toml` and `evidence.toml` to drop entries
//!   pointing at non-public threads. Body and node text bytes pass
//!   through unchanged (RFC §4).
//!
//! - [`lint`] — pre-publish lint per RFC §4.4. Pure scan over body
//!   and node text, reports tokens that name known-private threads.
//!   Informational; never rewrites content.
//! - [`commit`] — parentless single-commit construction and
//!   tree-equivalence idempotency (RFC §2).

pub mod commit;
pub mod exclusion;
pub mod lint;
pub mod orchestrate;
