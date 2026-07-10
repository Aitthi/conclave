use crate::engine::{AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt::Display;
use std::path::PathBuf;

const DEFAULT_LIMIT: usize = 200;
const DEFAULT_DEPTH: usize = 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootReq {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesReq {
    path: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolsReq {
    path: String,
    target: Option<String>,
    all: Option<bool>,
    kind: Option<Vec<String>>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindReq {
    path: String,
    name: String,
    exact: Option<bool>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraversalReq {
    path: String,
    name: String,
    depth: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefsReq {
    path: String,
    name: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamedReq {
    path: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameReq {
    path: String,
    old: String,
    new: String,
    apply: Option<bool>,
    lang: Option<String>,
    anchor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RewriteReq {
    path: String,
    pattern: String,
    rewrite: String,
    apply: Option<bool>,
    lang: Option<String>,
}

fn parse<T: for<'de> Deserialize<'de>>(payload: Value) -> Result<T, AppError> {
    serde_json::from_value::<T>(payload).map_err(|e| AppError::Invalid(e.to_string()))
}

fn root_of(raw: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(AppError::Invalid(format!(
            "code: path must be absolute, got {raw}"
        )));
    }
    if !path.is_dir() {
        return Err(AppError::Invalid(format!("code: not a directory: {raw}")));
    }
    Ok(path)
}

fn wrap(data: Value, truncated: bool, warnings: &[String]) -> Value {
    let mut value = codeintel::output::envelope(data);
    if truncated {
        value["truncated"] = json!(true);
    }
    if !warnings.is_empty() {
        value["warnings"] = json!(warnings);
    }
    value
}

fn code_error(error: impl Display) -> String {
    error.to_string()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("code: task join: {e}")))?
        .map_err(|e| AppError::Invalid(format!("code: {e}")))
}

pub async fn stats(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RootReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let data = codeintel::map::stats(&root, &idx).map_err(code_error)?;
        Ok((data, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}

pub async fn files(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: FilesReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let cache = state.code_cache.clone();
    let (data, truncated, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let (data, truncated) = codeintel::map::files(&idx, limit).map_err(code_error)?;
        Ok((data, truncated, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, truncated, &warnings))
}

pub async fn tree(_state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RootReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let data = blocking(move || codeintel::map::tree(&root).map_err(code_error)).await?;
    Ok(wrap(data, false, &[]))
}

pub async fn symbols(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SymbolsReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let target = req.target;
    let all = req.all.unwrap_or(false);
    let kinds = req.kind.unwrap_or_default();
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let cache = state.code_cache.clone();
    let (data, truncated, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let (data, truncated) =
            codeintel::map::symbols(&idx, target.as_deref(), all, &kinds, limit)
                .map_err(code_error)?;
        Ok((data, truncated, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, truncated, &warnings))
}

pub async fn find(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: FindReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let exact = req.exact.unwrap_or(false);
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let name = req.name;
    let cache = state.code_cache.clone();
    let (data, truncated, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let (data, truncated) =
            codeintel::map::find(&idx, &name, exact, limit).map_err(code_error)?;
        Ok((data, truncated, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, truncated, &warnings))
}

pub async fn callers(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: TraversalReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let depth = req.depth.unwrap_or(DEFAULT_DEPTH);
    let name = req.name;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let data = codeintel::graph::callers(&idx, &name, depth).map_err(code_error)?;
        Ok((data, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}

pub async fn callees(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: TraversalReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let depth = req.depth.unwrap_or(DEFAULT_DEPTH);
    let name = req.name;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let data = codeintel::graph::callees(&idx, &name, depth).map_err(code_error)?;
        Ok((data, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}

pub async fn refs(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RefsReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let name = req.name;
    let cache = state.code_cache.clone();
    let (data, truncated, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let (data, truncated) =
            codeintel::graph::find_refs(&idx, &name, limit).map_err(code_error)?;
        Ok((data, truncated, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, truncated, &warnings))
}

pub async fn impact(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: NamedReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let name = req.name;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let data = codeintel::graph::impact(&idx, &name).map_err(code_error)?;
        Ok((data, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}

pub async fn rename(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RenameReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root).map_err(code_error)?;
        let warnings = idx.warnings.clone();
        let (data, written) = codeintel::edit::rename(
            &root,
            &idx,
            &req.old,
            &req.new,
            req.apply.unwrap_or(false),
            req.lang.as_deref(),
            req.anchor.as_deref(),
        )
        .map_err(code_error)?;
        cache.invalidate_files(&root, &written);
        Ok((data, warnings))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}

pub async fn rewrite(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RewriteReq = parse(payload)?;
    let root = root_of(&req.path)?;
    let cache = state.code_cache.clone();
    let data = blocking(move || {
        let (data, written) = codeintel::edit::rewrite(
            &root,
            &req.pattern,
            &req.rewrite,
            req.apply.unwrap_or(false),
            req.lang.as_deref(),
        )
        .map_err(code_error)?;
        cache.invalidate_files(&root, &written);
        Ok(data)
    })
    .await?;
    Ok(wrap(data, false, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{AppError, AppState};
    use serde_json::json;

    #[tokio::test]
    async fn stats_rejects_missing_path() {
        let state = AppState::for_tests().await;
        let err = stats(&state, json!({})).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn stats_rejects_nonexistent_root() {
        let state = AppState::for_tests().await;
        let err = stats(&state, json!({"path": "/nonexistent/xyz"}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn find_returns_envelope_with_truncation_flag() {
        let state = AppState::for_tests().await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn alpha() {}\nfn alpha_two() {}\n",
        )
        .unwrap();
        let out = find(
            &state,
            json!({"path": dir.path(), "name": "alpha", "limit": 1}),
        )
        .await
        .unwrap();
        assert_eq!(out["schema_version"], 1);
        assert_eq!(out["data"].as_array().unwrap().len(), 1);
        assert_eq!(out["truncated"], true);
    }

    #[tokio::test]
    async fn rename_dry_run_touches_no_files() {
        let state = AppState::for_tests().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "fn old_name() {}\nfn c() { old_name(); }\n").unwrap();
        let out = rename(
            &state,
            json!({"path": dir.path(), "old": "old_name", "new": "new_name"}),
        )
        .await
        .unwrap();
        assert_eq!(out["data"]["dry_run"], true);
        assert!(std::fs::read_to_string(&file).unwrap().contains("old_name"));
    }
}
