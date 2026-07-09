//! Role command handlers — CRUD for first-class agent roles (ADR 0005).
//! Builtin-vs-custom AUTHORIZATION (rejecting a mutation of a builtin role, or
//! a custom role that collides with a builtin slug) is enforced HERE, not in
//! `repo::role` (which stays a plain CRUD + folder-reader mirror) — same
//! division of responsibility as `commands::skill`.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveRoleReq {
    id: Option<String>,
    name: String,
    description: String,
    #[serde(default)]
    skill_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteRoleReq {
    id: String,
}

/// True when `name` (case-insensitively) collides with a builtin role slug —
/// a custom role must never shadow one, since builtin ids ARE their slugs and
/// `find_any` resolves builtins first (a custom "Lead" would be unreachable by
/// id and confusing in the picker). See ADR 0005 risk ledger.
fn collides_with_builtin_slug(name: &str) -> bool {
    let trimmed = name.trim();
    repo::role::list_builtin()
        .iter()
        .any(|r| r.id.eq_ignore_ascii_case(trimmed) || r.name.eq_ignore_ascii_case(trimmed))
}

/// Return every role: builtin first (from the bundled folder — see
/// `repo::role::list_builtin` / ADR 0005), then every custom role (from the
/// DB, name-ordered). Maps to `role.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let builtins = repo::role::list_builtin();
    let customs = repo::role::list(&state.db).await?;

    let mut items = Vec::with_capacity(builtins.len() + customs.len());
    for r in builtins.iter().chain(customs.iter()) {
        items.push(serde_json::to_value(r).map_err(|e| AppError::Internal(e.to_string()))?);
    }
    Ok(Value::Array(items))
}

/// Create or update a CUSTOM role. Maps to `role.save` on the IPC bus.
/// Rejects (`AppError::Invalid`) an attempt to edit a builtin role (checked
/// against `list_builtin()` FIRST — a builtin id is never in the DB), and a
/// custom name that collides with a builtin slug.
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveRoleReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if req.name.trim().is_empty() {
        return Err(AppError::Invalid("role name must not be empty".into()));
    }
    if req.description.trim().is_empty() {
        return Err(AppError::Invalid(
            "role description must not be empty".into(),
        ));
    }
    if collides_with_builtin_slug(&req.name) {
        return Err(AppError::Invalid(
            "role name collides with a builtin role".into(),
        ));
    }

    if let Some(id) = req.id.as_deref() {
        if repo::role::list_builtin().iter().any(|r| r.id == id) {
            return Err(AppError::Invalid("cannot edit a builtin role".into()));
        }
        let row = repo::role::update(&state.db, id, &req.name, &req.description, &req.skill_ids)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("role id={id} not found")))?;
        return serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()));
    }

    let row = repo::role::create(&state.db, &req.name, &req.description, &req.skill_ids).await?;
    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete a CUSTOM role. Maps to `role.delete` on the IPC bus. Rejects
/// (`AppError::Invalid`) an attempt to delete a builtin role. Any
/// `agent_definition.role_id` referencing the deleted role is NULLed (the
/// display-text `role` label is left intact) — see `repo::role::delete`.
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DeleteRoleReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if repo::role::list_builtin().iter().any(|r| r.id == req.id) {
        return Err(AppError::Invalid("cannot delete a builtin role".into()));
    }

    repo::role::get(&state.db, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("role id={} not found", req.id)))?;
    repo::role::delete(&state.db, &req.id).await?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_creates_then_updates_custom_role() {
        let state = AppState::for_tests().await;
        let created = save(
            &state,
            serde_json::json!({
                "name": "My Reviewer",
                "description": "Reviews diffs.",
                "skillIds": ["implementer"]
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().expect("id present").to_owned();
        assert_eq!(created["kind"], "custom");
        assert_eq!(created["skillIds"], serde_json::json!(["implementer"]));

        let updated = save(
            &state,
            serde_json::json!({
                "id": id,
                "name": "My Reviewer 2",
                "description": "Reviews harder.",
                "skillIds": []
            }),
        )
        .await
        .expect("update failed");
        assert_eq!(updated["name"], "My Reviewer 2");
        assert_eq!(updated["skillIds"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn save_rejects_editing_builtin() {
        let _fx = repo::role::test_support::fixture_roles_dir("cmd-save-rejects-builtin");
        let state = AppState::for_tests().await;
        let builtin_id = repo::role::list_builtin()
            .first()
            .expect("fixture must yield a builtin")
            .id
            .clone();
        let result = save(
            &state,
            serde_json::json!({ "id": builtin_id, "name": "Hacked", "description": "x" }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Invalid(_))));
    }

    #[tokio::test]
    async fn save_rejects_name_colliding_with_builtin_slug() {
        let _fx = repo::role::test_support::fixture_roles_dir("cmd-save-rejects-collision");
        let state = AppState::for_tests().await;
        // Fixture builtin id "fix-lead" / name "Fixture Lead" — both must be
        // rejected case-insensitively.
        for name in ["fix-lead", "FIX-LEAD", "Fixture Lead"] {
            let result = save(
                &state,
                serde_json::json!({ "name": name, "description": "shadow attempt" }),
            )
            .await;
            assert!(
                matches!(result, Err(AppError::Invalid(_))),
                "name '{name}' must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn save_rejects_empty_name_or_description() {
        let state = AppState::for_tests().await;
        assert!(matches!(
            save(
                &state,
                serde_json::json!({ "name": "  ", "description": "d" })
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            save(
                &state,
                serde_json::json!({ "name": "X", "description": "" })
            )
            .await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn delete_rejects_builtin_but_allows_custom() {
        let _fx = repo::role::test_support::fixture_roles_dir("cmd-delete-rejects-builtin");
        let state = AppState::for_tests().await;
        let builtin_id = repo::role::list_builtin()
            .first()
            .expect("fixture must yield a builtin")
            .id
            .clone();
        let created = save(
            &state,
            serde_json::json!({ "name": "Custom", "description": "c" }),
        )
        .await
        .expect("create failed");
        let custom_id = created["id"].as_str().unwrap().to_owned();

        let builtin_delete = delete(&state, serde_json::json!({ "id": builtin_id })).await;
        assert!(matches!(builtin_delete, Err(AppError::Invalid(_))));

        let custom_delete = delete(&state, serde_json::json!({ "id": custom_id })).await;
        assert!(custom_delete.is_ok());
    }

    #[tokio::test]
    async fn list_includes_builtin_and_custom_builtin_first() {
        let _fx = repo::role::test_support::fixture_roles_dir("cmd-list-builtin-custom");
        let state = AppState::for_tests().await;
        save(
            &state,
            serde_json::json!({ "name": "Zzz Custom", "description": "c" }),
        )
        .await
        .expect("create failed");

        let listed = list(&state, Value::Null).await.expect("list failed");
        let arr = listed.as_array().unwrap();
        assert!(
            arr.iter()
                .any(|r| r["kind"] == "builtin" && r["id"] == "fix-lead"),
            "builtin fixture role must appear"
        );
        assert!(
            arr.iter()
                .any(|r| r["kind"] == "custom" && r["name"] == "Zzz Custom"),
            "custom role must appear"
        );
        // Builtins come before customs.
        let first_custom = arr.iter().position(|r| r["kind"] == "custom").unwrap();
        let last_builtin = arr.iter().rposition(|r| r["kind"] == "builtin").unwrap();
        assert!(last_builtin < first_custom, "builtins must precede customs");
    }
}
