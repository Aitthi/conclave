use crate::engine::commands::{
    agent, blackboard, cli, fusion, instance, message, provider, snapshot, workspace,
};
use crate::engine::{AppError, AppState};
use serde_json::Value;

/// Central command dispatcher — the single chokepoint for all IPC traffic.
///
/// Both the Tauri `invoke` bridge (now) and the future Unix-Domain-Socket
/// server (later milestone) funnel through here.
pub async fn dispatch(state: &AppState, cmd: &str, payload: Value) -> Result<Value, AppError> {
    match cmd {
        // ── workspace ──────────────────────────────────────────────────────
        "workspace.list" => workspace::list(state, payload).await,
        "workspace.link" => workspace::link(state, payload).await,
        "workspace.use" => workspace::use_workspace(state, payload).await,

        // ── agentDef ──────────────────────────────────────────────────────
        "agentDef.list" => agent::list(state, payload).await,
        "agentDef.save" => agent::save(state, payload).await,
        "agentDef.addToWorkspace" => agent::add_to_workspace(state, payload).await,

        // ── instance ──────────────────────────────────────────────────────
        "instance.list" => instance::list(state, payload).await,
        "instance.spawn" => instance::spawn(state, payload).await,

        // ── message ───────────────────────────────────────────────────────
        "message.send" => message::send(state, payload).await,
        "message.inject" => message::inject(state, payload).await,
        "message.list" => message::list(state, payload).await,

        // ── blackboard ────────────────────────────────────────────────────
        "blackboard.list" => blackboard::list(state, payload).await,
        "blackboard.get" => blackboard::get(state, payload).await,
        "blackboard.set" => blackboard::set(state, payload).await,

        // ── snapshot ──────────────────────────────────────────────────────
        "snapshot.create" => snapshot::create(state, payload).await,
        "snapshot.list" => snapshot::list(state, payload).await,
        "snapshot.read" => snapshot::read(state, payload).await,

        // ── fusion ────────────────────────────────────────────────────────
        "fusion.run" => fusion::run(state, payload).await,

        // ── provider ──────────────────────────────────────────────────────
        "provider.upsert" => provider::upsert(state, payload).await,

        // ── cli ───────────────────────────────────────────────────────────
        "cli.exec" => cli::exec(state, payload).await,

        // ── unknown ───────────────────────────────────────────────────────
        other => Err(AppError::NotFound(format!("unknown command: {other}"))),
    }
}
