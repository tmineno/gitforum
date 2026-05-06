//! Publish pipeline for `git forum push` (RFC `fls856j3`).
//!
//! - [`exclusion`] — pure transformation that filters a public
//!   thread's `links.toml` and `evidence.toml` to drop entries
//!   pointing at non-public threads. Body and node text bytes pass
//!   through unchanged (RFC §4).
//!
//! Forthcoming submodules: `lint` (pre-publish lint per RFC §4.4),
//! `commit` (parentless single-commit construction per §2),
//! `withdrawal` (RFC §7).

pub mod exclusion;
