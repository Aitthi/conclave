//! Provider command handlers — upsert and list LLM provider configurations.
//!
//! SECURITY: the raw API key travels from the IPC payload directly into the
//! macOS Keychain (`engine::secrets::set_key`) and is NEVER written to the
//! database, echoed in a log, included in an error message, or returned in any
//! IPC response. The database stores only a masked display value produced by
//! `mask_key`. Every IPC response contains `ProviderRow` (masked data only).

use crate::engine::{repo, secrets, AppError, AppState};
use repo::provider::ProviderRow;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

// ── Key masking ──────────────────────────────────────────────────────────────

/// Produce a display-safe mask for a raw API key.
///
/// - Keys of ≤ 12 characters → `"••••"` (length is not revealed; a
///   just-over-threshold key would otherwise expose all but a few of its chars).
/// - Longer keys → first 4 chars + `"…"` + last 4 chars.
///
/// Uses char-boundary–safe slicing so non-ASCII keys are handled without panic.
pub(crate) fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        "••••".to_owned()
    } else {
        let first: String = chars[..4].iter().collect();
        let last: String = chars[chars.len() - 4..].iter().collect();
        format!("{first}…{last}")
    }
}

// ── Request type ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertReq {
    name: String,
    /// Raw API key — written to the Keychain only; NEVER logged or returned.
    key: Option<String>,
    base_url: Option<String>,
}

// Manual `Debug` that REDACTS the raw key — auto-deriving `Debug` would print
// the secret if this struct ever reached a log line, panic report, or `dbg!`.
impl std::fmt::Debug for UpsertReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpsertReq")
            .field("name", &self.name)
            .field("key", &self.key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `provider.upsert` — create or update an LLM provider configuration.
///
/// Steps:
/// 1. Validate `name` ∈ {anthropic, openai, local, custom}.
/// 2. Look up any existing row to carry forward `id`, `masked_key`, `status`.
/// 3. If a non-empty `key` is present, write it to the Keychain and compute
///    the masked display value. The real key never leaves this scope.
/// 4. Merge `base_url` (payload value takes precedence over existing).
/// 5. Upsert the row (masked data only) and return it.
pub async fn upsert(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: UpsertReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Validate provider name. "custom" is intentionally NOT accepted yet: the
    // chat runtime (`from_config`) has no `custom` arm, so accepting it would
    // store a key the runtime can never use. Deferred until a custom-endpoint
    // runtime arm exists.
    match req.name.as_str() {
        "anthropic" | "openai" | "local" => {}
        other => {
            return Err(AppError::Invalid(format!(
                "unknown provider name: {other}; expected anthropic, openai, or local"
            )))
        }
    }

    // Carry-forward existing row (id + masked_key + status).
    let existing = repo::provider::get_by_name(&state.db, &req.name).await?;

    let id = existing
        .as_ref()
        .map(|e| e.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Handle the key: Keychain write + masking. The real key is dropped at the
    // end of this block — it never reaches ProviderRow or the response.
    // Keychain account = the provider NAME (unique: one row per name). This MUST
    // match the account `runtime::provider::from_config` reads by — it resolves
    // keys via `secrets::get_key(provider_id)` where `provider_id` is the name
    // ("anthropic"/"openai"/…). Keying the Keychain by the row UUID instead would
    // store a key the chat runtime could never find.
    let masked_key = match req.key.as_deref().filter(|k| !k.trim().is_empty()) {
        Some(key) => {
            secrets::set_key(&req.name, key)
                .map_err(|_| AppError::Internal("failed to save key to Keychain".into()))?;
            Some(mask_key(key))
        }
        None => existing.as_ref().and_then(|e| e.masked_key.clone()),
    };

    let base_url = req
        .base_url
        .filter(|s| !s.trim().is_empty())
        .or_else(|| existing.as_ref().and_then(|e| e.base_url.clone()));

    // Carry forward status; do NOT fabricate "connected" (no connectivity test).
    let status = existing.as_ref().and_then(|e| e.status.clone());

    let row = ProviderRow {
        id,
        name: req.name,
        base_url,
        masked_key,
        stored_in: "keychain".to_owned(),
        status,
    };

    let saved = repo::provider::upsert(&state.db, &row).await?;
    serde_json::to_value(&saved).map_err(|e| AppError::Internal(e.to_string()))
}

/// `provider.list` — return all configured providers (masked keys only).
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let rows = repo::provider::list(&state.db).await?;
    serde_json::to_value(&rows).map_err(|e| AppError::Internal(e.to_string()))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── mask_key ─────────────────────────────────────────────────────────────

    #[test]
    fn mask_key_short_keys_become_bullets() {
        assert_eq!(mask_key(""), "••••");
        assert_eq!(mask_key("abc"), "••••");
        assert_eq!(mask_key("12345678"), "••••"); // exactly 8 chars → bullets
    }

    #[test]
    fn mask_key_long_keys_show_prefix_and_suffix() {
        // 9+ chars → first 4 + "…" + last 4
        assert_eq!(mask_key("sk-abcdefghij"), "sk-a…ghij");
        let m = mask_key("AAAA_secret_BBBB");
        assert!(m.starts_with("AAAA"), "should start with first 4 chars");
        assert!(m.ends_with("BBBB"), "should end with last 4 chars");
        assert!(m.contains('…'), "should contain the ellipsis character");
        assert!(!m.contains("secret"), "middle of the key must not appear");
    }

    #[test]
    fn mask_key_non_ascii_safe() {
        // 6-char string: ≤ 12 → bullets
        assert_eq!(mask_key("日本語テスト"), "••••");
        // 9-char non-ASCII string: still ≤ 12 → bullets (no char-boundary panic)
        assert_eq!(mask_key("αβγδεζηθι"), "••••");
        // 13-char non-ASCII string → first 4 + … + last 4
        assert_eq!(mask_key("αβγδεζηθικλμν"), "αβγδ…κλμν");
    }

    // ── upsert (no key — must NOT touch the Keychain) ────────────────────────

    #[tokio::test]
    async fn upsert_base_url_only_no_keychain() {
        let state = AppState::for_tests().await;

        let result = upsert(
            &state,
            json!({ "name": "anthropic", "baseUrl": "https://api.anthropic.com" }),
        )
        .await
        .expect("upsert without key should succeed");

        assert_eq!(
            result.get("name").and_then(|v| v.as_str()),
            Some("anthropic")
        );
        assert_eq!(
            result.get("storedIn").and_then(|v| v.as_str()),
            Some("keychain")
        );
        assert_eq!(
            result.get("baseUrl").and_then(|v| v.as_str()),
            Some("https://api.anthropic.com")
        );
        // No key provided → maskedKey must be absent.
        assert!(
            result.get("maskedKey").is_none(),
            "maskedKey must be absent when no key was provided"
        );
    }

    #[tokio::test]
    async fn list_after_upsert_returns_saved_row() {
        let state = AppState::for_tests().await;

        upsert(
            &state,
            json!({ "name": "openai", "baseUrl": "https://api.openai.com/v1" }),
        )
        .await
        .unwrap();

        let result = list(&state, Value::Null).await.unwrap();
        let arr = result.as_array().expect("list must return a JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("openai"));
    }

    #[tokio::test]
    async fn upsert_rejects_invalid_name() {
        let state = AppState::for_tests().await;

        let err = upsert(&state, json!({ "name": "unknown_provider" }))
            .await
            .expect_err("invalid name should be rejected");
        assert!(matches!(err, AppError::Invalid(_)));
    }
}
