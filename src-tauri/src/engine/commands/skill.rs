//! Skill command handlers — CRUD for reusable prompt modules attached to
//! `AgentDefinition`s. Builtin-vs-custom AUTHORIZATION (rejecting a mutation
//! of a `kind = "builtin"` row) is enforced HERE, not in `repo::skill` (which
//! stays a plain CRUD mirror of `agent_definition`'s pattern) — same division
//! of responsibility as the `permission_mode` allowlist check living in
//! `commands::agent`, not the repo layer.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSkillReq {
    id: Option<String>,
    name: String,
    description: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSkillReq {
    id: String,
}

/// Return every skill (builtin first, from the bundled folder — see
/// `repo::skill::list_builtin` / ADR 0002), then every custom skill (from the
/// DB), each custom item annotated with `attachedTo`: how many agent
/// definitions currently have it attached. Builtin items carry no
/// `attachedTo` key at all (the frontend's system-skill card never reads it).
///
/// Maps to `skill.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let builtins = repo::skill::list_builtin();
    let customs = repo::skill::list(&state.db).await?;
    let counts = repo::skill::attached_counts(&state.db).await?;

    let mut items = Vec::with_capacity(builtins.len() + customs.len());
    for s in &builtins {
        items.push(serde_json::to_value(s).map_err(|e| AppError::Internal(e.to_string()))?);
    }
    for s in &customs {
        let mut v = serde_json::to_value(s).map_err(|e| AppError::Internal(e.to_string()))?;
        v["attachedTo"] = serde_json::json!(counts.get(&s.id).copied().unwrap_or(0));
        items.push(v);
    }
    Ok(Value::Array(items))
}

/// Create or update a CUSTOM skill. Maps to `skill.save` on the IPC bus.
/// Rejects (`AppError::Invalid`) an attempt to edit a builtin skill — checked
/// against `repo::skill::list_builtin()` FIRST, since a builtin id is never
/// in the DB (a bare `repo::skill::get` would incorrectly report `NotFound`).
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if let Some(id) = req.id.as_deref() {
        if repo::skill::list_builtin().iter().any(|s| s.id == id) {
            return Err(AppError::Invalid("cannot edit a builtin skill".into()));
        }
        let existing = repo::skill::get(&state.db, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        let _ = existing; // presence already confirmed; row itself isn't needed further
        let row = repo::skill::update(
            &state.db,
            id,
            &req.name,
            req.description.as_deref(),
            &req.content,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        return serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()));
    }

    let row = repo::skill::create(
        &state.db,
        &req.name,
        req.description.as_deref(),
        &req.content,
    )
    .await?;
    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete a CUSTOM skill. Maps to `skill.delete` on the IPC bus. Rejects
/// (`AppError::Invalid`) an attempt to delete a builtin skill, checked
/// against `repo::skill::list_builtin()` first (same reasoning as `save`).
/// `agent_skill` rows referencing it cascade away (`ON DELETE CASCADE`).
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DeleteSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if repo::skill::list_builtin().iter().any(|s| s.id == req.id) {
        return Err(AppError::Invalid("cannot delete a builtin skill".into()));
    }

    repo::skill::get(&state.db, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("skill id={} not found", req.id)))?;
    repo::skill::delete(&state.db, &req.id).await?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_creates_then_updates_custom_skill() {
        let state = AppState::for_tests().await;
        let created = save(
            &state,
            serde_json::json!({ "name": "Reviewer", "description": "desc", "content": "text" }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().expect("id present").to_owned();

        let updated = save(
            &state,
            serde_json::json!({ "id": id, "name": "Reviewer2", "content": "new text" }),
        )
        .await
        .expect("update failed");
        assert_eq!(updated["name"], "Reviewer2");
        assert_eq!(updated["content"], "new text");
    }

    #[tokio::test]
    async fn save_rejects_editing_builtin() {
        let state = AppState::for_tests().await;
        let builtin_id = repo::skill::list_builtin()
            .first()
            .expect("at least the checked-in example skill must exist")
            .id
            .clone();

        let result = save(
            &state,
            serde_json::json!({ "id": builtin_id, "name": "Hacked", "content": "x" }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Invalid(_))));
    }

    #[tokio::test]
    async fn delete_rejects_builtin_but_allows_custom() {
        let state = AppState::for_tests().await;
        let builtin_id = repo::skill::list_builtin()
            .first()
            .expect("at least the checked-in example skill must exist")
            .id
            .clone();
        let created = save(&state, serde_json::json!({ "name": "Custom", "content": "c" }))
            .await
            .expect("create failed");
        let custom_id = created["id"].as_str().unwrap().to_owned();

        let builtin_delete = delete(&state, serde_json::json!({ "id": builtin_id })).await;
        assert!(matches!(builtin_delete, Err(AppError::Invalid(_))));

        let custom_delete = delete(&state, serde_json::json!({ "id": custom_id })).await;
        assert!(custom_delete.is_ok());
    }

    #[tokio::test]
    async fn list_includes_builtin_and_custom() {
        let state = AppState::for_tests().await;
        save(&state, serde_json::json!({ "name": "Custom", "content": "c" }))
            .await
            .expect("create failed");

        let listed = list(&state, Value::Null).await.expect("list failed");
        let arr = listed.as_array().unwrap();
        assert!(
            arr.iter().any(|s| s["kind"] == "builtin" && s["id"] == "example"),
            "builtin example skill must appear in list()"
        );
        assert!(
            arr.iter().any(|s| s["kind"] == "custom" && s["name"] == "Custom"),
            "custom skill must appear in list()"
        );
        let builtin_item = arr.iter().find(|s| s["id"] == "example").unwrap();
        assert!(
            builtin_item.get("attachedTo").is_none(),
            "builtin items must not carry an attachedTo annotation"
        );
    }

    #[tokio::test]
    async fn list_annotates_attached_to_count() {
        let state = AppState::for_tests().await;
        let created = save(&state, serde_json::json!({ "name": "S", "content": "c" }))
            .await
            .expect("create failed");
        let skill_id = created["id"].as_str().unwrap().to_owned();

        let def = repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        repo::skill::set_custom_attachments(&state.db, &def.id, std::slice::from_ref(&skill_id))
            .await
            .expect("attach failed");

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == skill_id)
            .expect("skill present");
        assert_eq!(item["attachedTo"], 1);
    }
}
