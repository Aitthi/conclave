use crate::engine::commands::{
    agent, blackboard, cli, fusion, instance, memory, message, provider, role, skill, skill_draft,
    snapshot, tool, workspace,
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
        "workspace.update" => workspace::update(state, payload).await,
        "workspace.delete" => workspace::delete(state, payload).await,

        // ── agentDef ──────────────────────────────────────────────────────
        "agentDef.list" => agent::list(state, payload).await,
        "agentDef.save" => agent::save(state, payload).await,
        "agentDef.delete" => agent::delete(state, payload).await,
        "agentDef.addToWorkspace" => agent::add_to_workspace(state, payload).await,

        // ── skill ─────────────────────────────────────────────────────────
        "skill.list" => skill::list(state, payload).await,
        "skill.save" => skill::save(state, payload).await,
        "skill.delete" => skill::delete(state, payload).await,
        "skill.startDraftSession" => skill_draft::start(state, payload).await,
        "skill.syncDraft" => skill_draft::sync(state, payload).await,
        "skill.stopDraftSession" => skill_draft::stop(state, payload).await,

        // ── role ──────────────────────────────────────────────────────────
        "role.list" => role::list(state, payload).await,
        "role.save" => role::save(state, payload).await,
        "role.delete" => role::delete(state, payload).await,

        // ── instance ──────────────────────────────────────────────────────
        "instance.list" => instance::list(state, payload).await,
        "instance.spawn" => instance::spawn(state, payload).await,
        "instance.restart" => instance::restart(state, payload).await,
        "instance.remove" => instance::remove(state, payload).await,
        "session.resize" => instance::resize(state, payload).await,

        // ── message ───────────────────────────────────────────────────────
        "message.send" => message::send(state, payload).await,
        "message.inject" => message::inject(state, payload).await,
        "message.list" => message::list(state, payload).await,
        "message.listForWorkspace" => message::list_for_workspace(state, payload).await,

        // ── blackboard ────────────────────────────────────────────────────
        "blackboard.list" => blackboard::list(state, payload).await,
        "blackboard.get" => blackboard::get(state, payload).await,
        "blackboard.set" => blackboard::set(state, payload).await,
        "blackboard.delete" => blackboard::delete(state, payload).await,

        // ── memory ────────────────────────────────────────────────────────
        "memory.remember" => memory::remember(state, payload).await,
        "memory.search" => memory::search(state, payload).await,
        "memory.delete" => memory::delete(state, payload).await,
        "memory.clear" => memory::clear(state, payload).await,
        "memory.status" => memory::status(state, payload).await,
        "memory.graph" => memory::graph(state, payload).await,

        // ── snapshot ──────────────────────────────────────────────────────
        "snapshot.create" => snapshot::create(state, payload).await,
        "snapshot.list" => snapshot::list(state, payload).await,
        "snapshot.read" => snapshot::read(state, payload).await,
        "snapshot.save" => snapshot::save(state, payload).await,
        "snapshot.last" => snapshot::last(state, payload).await,
        "snapshot.compact" => snapshot::compact(state, payload).await,
        "snapshot.resume" => snapshot::resume(state, payload).await,
        "snapshot.delete" => snapshot::delete(state, payload).await,
        "snapshot.send" => snapshot::send(state, payload).await,

        // ── fusion ────────────────────────────────────────────────────────
        "fusion.run" => fusion::run(state, payload).await,
        "fusion.get" => fusion::get(state, payload).await,

        // ── provider ──────────────────────────────────────────────────────
        "provider.upsert" => provider::upsert(state, payload).await,
        "provider.list" => provider::list(state, payload).await,

        // ── tool ──────────────────────────────────────────────────────────
        "tool.list" => tool::list(state, payload).await,

        // ── cli ───────────────────────────────────────────────────────────
        "cli.exec" => cli::exec(state, payload).await,

        // ── unknown ───────────────────────────────────────────────────────
        other => Err(AppError::NotFound(format!("unknown command: {other}"))),
    }
}
