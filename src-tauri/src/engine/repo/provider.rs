//! Provider repository — LLM provider configuration stored in the `provider`
//! table.
//!
//! The real API key is NEVER stored here. Only a masked display value
//! (`masked_key`) is persisted so the Settings screen can show "Key saved"
//! without exposing the credential. The real key lives in the macOS Keychain
//! (see `engine::secrets`).
//!
//! # chain-builder note
//!
//! `upsert` uses raw sqlx (`sqlx::query_as`) because the
//! `ON CONFLICT(id) DO UPDATE SET ... = excluded.*` syntax cannot be expressed
//! through chain-builder's value-only `insert`. This is the documented
//! fallback (see `repo/mod.rs`).

use serde::Serialize;
use sqlx::SqlitePool;

// ── Row struct ───────────────────────────────────────────────────────────────

/// Decoded row from the `provider` table.
///
/// `sqlx::FromRow` maps snake_case columns to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `Provider` interface in `src/ipc/types.ts`. Optional columns use
/// `skip_serializing_if` so an absent value is a missing key (matching
/// `field?: T` TypeScript syntax), never `null`.
///
/// `masked_key` holds a display-safe mask (e.g. `"sk-ab…cdef"`). The real
/// key is NEVER present in this struct.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masked_key: Option<String>,
    pub stored_in: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

// ── CRUD ─────────────────────────────────────────────────────────────────────

/// Look up a provider row by its `name` (e.g. `"anthropic"`).
///
/// `LIMIT 1` keeps `fetch_optional` safe even if a future race ever produced
/// two rows for the same name (the `provider` table has no UNIQUE(name)
/// constraint) — it returns the first rather than erroring.
pub async fn get_by_name(pool: &SqlitePool, name: &str) -> sqlx::Result<Option<ProviderRow>> {
    sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, base_url, masked_key, stored_in, status \
         FROM provider \
         WHERE name = ? \
         LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

/// Insert or update a provider row (conflict on `id`).
///
/// All mutable columns are overwritten on conflict. `stored_in` is always
/// `"keychain"` per the table check constraint. Returns the post-upsert row.
pub async fn upsert(pool: &SqlitePool, row: &ProviderRow) -> sqlx::Result<ProviderRow> {
    sqlx::query_as::<_, ProviderRow>(
        "INSERT INTO provider(id, name, base_url, masked_key, stored_in, status) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           name       = excluded.name, \
           base_url   = excluded.base_url, \
           masked_key = excluded.masked_key, \
           stored_in  = excluded.stored_in, \
           status     = excluded.status \
         RETURNING id, name, base_url, masked_key, stored_in, status",
    )
    .bind(&row.id)
    .bind(&row.name)
    .bind(&row.base_url)
    .bind(&row.masked_key)
    .bind(&row.stored_in)
    .bind(&row.status)
    .fetch_one(pool)
    .await
}

/// List all configured providers ordered by name.
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<ProviderRow>> {
    sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, base_url, masked_key, stored_in, status \
         FROM provider \
         ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn make_row(id: &str, name: &str, base_url: Option<&str>, masked: Option<&str>) -> ProviderRow {
        ProviderRow {
            id: id.to_owned(),
            name: name.to_owned(),
            base_url: base_url.map(str::to_owned),
            masked_key: masked.map(str::to_owned),
            stored_in: "keychain".to_owned(),
            status: None,
        }
    }

    #[tokio::test]
    async fn upsert_insert_then_get_by_name() {
        let pool = connect_in_memory().await;

        let row = make_row(
            "prov-1",
            "anthropic",
            Some("https://api.anthropic.com"),
            Some("sk-ab…cdef"),
        );
        upsert(&pool, &row).await.unwrap();

        let found = get_by_name(&pool, "anthropic").await.unwrap();
        assert!(found.is_some(), "provider must be findable by name");
        let found = found.unwrap();
        assert_eq!(found.id, "prov-1");
        assert_eq!(found.masked_key, Some("sk-ab…cdef".to_owned()));
        // The masked_key must not be the real credential.
        assert_ne!(
            found.masked_key.as_deref(),
            Some("real-secret-value"),
            "real key must not appear in the row"
        );
    }

    #[tokio::test]
    async fn upsert_update_changes_base_url() {
        let pool = connect_in_memory().await;

        upsert(
            &pool,
            &make_row(
                "prov-1",
                "anthropic",
                Some("https://api.anthropic.com"),
                None,
            ),
        )
        .await
        .unwrap();

        let updated = make_row(
            "prov-1",
            "anthropic",
            Some("https://custom.anthropic.com"),
            None,
        );
        let saved = upsert(&pool, &updated).await.unwrap();
        assert_eq!(
            saved.base_url,
            Some("https://custom.anthropic.com".to_owned())
        );
    }

    #[tokio::test]
    async fn list_returns_all_ordered_by_name() {
        let pool = connect_in_memory().await;

        upsert(&pool, &make_row("p2", "openai", None, None))
            .await
            .unwrap();
        upsert(&pool, &make_row("p1", "anthropic", None, None))
            .await
            .unwrap();

        let rows = list(&pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "anthropic");
        assert_eq!(rows[1].name, "openai");
    }

    /// JSON serialisation must use camelCase — locks the TypeScript `Provider`
    /// contract. Also verifies masked_key round-trips and None fields are omitted.
    #[test]
    fn camel_case_contract() {
        let row = ProviderRow {
            id: "p1".to_owned(),
            name: "anthropic".to_owned(),
            base_url: Some("https://api.anthropic.com".to_owned()),
            masked_key: Some("sk-ab…cdef".to_owned()),
            stored_in: "keychain".to_owned(),
            status: Some("connected".to_owned()),
        };
        let json = serde_json::to_value(&row).unwrap();

        // camelCase keys present.
        assert!(json.get("baseUrl").is_some(), "baseUrl must be camelCase");
        assert!(
            json.get("maskedKey").is_some(),
            "maskedKey must be camelCase"
        );
        assert!(json.get("storedIn").is_some(), "storedIn must be camelCase");

        // snake_case must NOT appear.
        assert!(json.get("base_url").is_none(), "no snake_case base_url");
        assert!(json.get("masked_key").is_none(), "no snake_case masked_key");
        assert!(json.get("stored_in").is_none(), "no snake_case stored_in");

        // masked_key round-trips correctly.
        assert_eq!(
            json.get("maskedKey").and_then(|v| v.as_str()),
            Some("sk-ab…cdef")
        );

        // None fields are absent (skip_serializing_if).
        let row_empty = ProviderRow {
            id: "p2".to_owned(),
            name: "openai".to_owned(),
            base_url: None,
            masked_key: None,
            stored_in: "keychain".to_owned(),
            status: None,
        };
        let json2 = serde_json::to_value(&row_empty).unwrap();
        assert!(json2.get("baseUrl").is_none(), "absent baseUrl omitted");
        assert!(json2.get("maskedKey").is_none(), "absent maskedKey omitted");
        assert!(json2.get("status").is_none(), "absent status omitted");
    }
}
