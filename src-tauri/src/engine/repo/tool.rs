//! Tool repository — core and plugin tool registry.
//!
//! Currently read-only: all tools are seeded by migrations (see
//! `0002_seed_core_tools.sql`). Write operations (register plugin / MCP tool)
//! are a later milestone.
//!
//! # chain-builder note
//!
//! `list_all` uses raw sqlx (`sqlx::query_as`) because the `COALESCE` over
//! `is_core` cannot be expressed as a simple column reference in chain-builder's
//! column list. This is the documented fallback (see `repo/mod.rs`).

use serde::Serialize;
use sqlx::SqlitePool;

// ── Row struct ───────────────────────────────────────────────────────────────

/// Decoded row from the `tool` table.
///
/// `sqlx::FromRow` maps snake_case columns to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `Tool` interface in `src/ipc/types.ts`. The `icon` field uses
/// `skip_serializing_if` so an absent icon is a missing key (matching
/// `icon?: string` in TypeScript), never `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub is_core: bool, // → "isCore"; sqlx decodes INTEGER 0/1 → bool
}

// ── CRUD ─────────────────────────────────────────────────────────────────────

/// List every registered tool, core built-ins first, then alphabetically.
///
/// `COALESCE(is_core, 0)` normalises NULL to 0 so tools without an explicit
/// `is_core` value sort correctly and sqlx can decode the column as `bool`
/// without encountering NULL.
pub async fn list_all(pool: &SqlitePool) -> sqlx::Result<Vec<ToolRow>> {
    sqlx::query_as::<_, ToolRow>(
        "SELECT id, name, kind, icon, COALESCE(is_core, 0) AS is_core \
         FROM tool \
         ORDER BY is_core DESC, name ASC",
    )
    .fetch_all(pool)
    .await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    /// After migration the seeded `tool-conclave` row must appear in `list_all`
    /// with `is_core == true` and `kind == "builtin"`.
    #[tokio::test]
    async fn list_all_returns_seeded_conclave_tool() {
        let pool = connect_in_memory().await;

        let tools = list_all(&pool)
            .await
            .expect("list_all should succeed on a fresh pool");

        assert!(
            !tools.is_empty(),
            "at least one tool must exist after migration"
        );

        let conclave = tools
            .iter()
            .find(|t| t.id == "tool-conclave")
            .expect("tool-conclave must be present");

        assert_eq!(conclave.name, "Conclave");
        assert_eq!(conclave.kind, "builtin");
        assert!(conclave.is_core, "conclave tool must be core");
        assert_eq!(conclave.icon.as_deref(), Some("terminal"));
    }

    /// JSON serialisation must use camelCase — locks the TypeScript `Tool`
    /// contract. Also verifies that a missing `icon` is absent (not null).
    #[tokio::test]
    async fn camel_case_contract() {
        let row = ToolRow {
            id: "tool-conclave".into(),
            name: "Conclave".into(),
            kind: "builtin".into(),
            icon: Some("terminal".into()),
            is_core: true,
        };

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase key present AND carries the correct boolean value
        assert_eq!(
            json.get("isCore").and_then(|v| v.as_bool()),
            Some(true),
            "isCore must serialize as true"
        );
        // snake_case must NOT appear
        assert!(json.get("is_core").is_none(), "no is_core key");

        // icon is Some → present
        assert_eq!(json.get("icon").and_then(|v| v.as_str()), Some("terminal"));

        // None icon → key absent
        let row_no_icon = ToolRow {
            id: "tool-x".into(),
            name: "X".into(),
            kind: "builtin".into(),
            icon: None,
            is_core: false,
        };
        let json2 = serde_json::to_value(&row_no_icon).expect("serialize failed");
        assert!(json2.get("icon").is_none(), "icon absent when None");
        assert_eq!(
            json2.get("isCore").and_then(|v| v.as_bool()),
            Some(false),
            "isCore must serialize as false"
        );
    }
}
