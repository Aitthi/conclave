//! Database repositories — one sub-module per domain entity.
//!
//! # Pattern (mirror this for every new entity)
//!
//! Each sub-module:
//!   1. Defines a `*Row` struct with `sqlx::FromRow + serde::Serialize` and
//!      `#[serde(rename_all = "camelCase")]` so snake_case DB columns become
//!      camelCase JSON across the Tauri IPC boundary.
//!   2. Exposes async CRUD fns that take `&SqlitePool` and return
//!      `sqlx::Result<T>`.  The pool is `Clone + Send + Sync` — no locking.
//!   3. Uses chain-builder for SELECT / INSERT query building (preferred);
//!      falls back to raw `sqlx::query` only where chain-builder is awkward
//!      (see individual module comments).
//!   4. Has a `#[cfg(test)]` block covering create/get roundtrip, list,
//!      exists, and a camelCase serialization contract test.
//!
//! Milestones that add new entities should copy `workspace.rs` as a template.

pub mod agent_definition;
pub mod blackboard;
pub mod fusion;
pub mod inter_agent_message;
pub mod provider;
pub mod session;
pub mod snapshot;
pub mod tool;
pub mod workspace;
pub mod workspace_agent;

/// Map a `chain_builder::Error` to a `sqlx::Error` so repo fns can return the
/// canonical `sqlx::Result<T>` and handlers can `?` into `AppError`.
///
/// Shared by every repo module (do NOT duplicate per-module). `Build` errors
/// (invalid query construction — a programmer mistake) map to
/// `sqlx::Error::Protocol` with a `[chain-builder]` prefix so they're
/// distinguishable in logs. The wildcard arm is required because
/// `chain_builder::Error` is `#[non_exhaustive]`.
pub(super) fn cb_err(e: chain_builder::Error) -> sqlx::Error {
    match e {
        chain_builder::Error::Sqlx(se) => se,
        chain_builder::Error::Build(be) => sqlx::Error::Protocol(format!("[chain-builder] {be}")),
        _ => sqlx::Error::Protocol("[chain-builder] unexpected error".into()),
    }
}
