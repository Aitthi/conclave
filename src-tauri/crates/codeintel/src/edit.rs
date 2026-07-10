//! AST-validated rename and structural rewrite verbs, ported from the
//! standalone `astedit` crate (see codeintel Task 5 brief). Unlike the
//! original CLI, these functions never print JSON or return a process exit
//! code — every condition (including the old exit-code-2 cases:
//! `needs_anchor`, pattern-compile failure) is represented in the returned
//! `serde_json::Value` payload, and callers always get `Ok` unless something
//! genuinely unexpected happened (e.g. `anyhow` plumbing failures).
pub mod apply;
pub mod error;
pub mod rewrite;
pub mod serialize;
mod rename_cmd;
mod rewrite_cmd;

pub use rename_cmd::rename;
pub use rewrite_cmd::rewrite;
