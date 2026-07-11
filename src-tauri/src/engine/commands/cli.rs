//! `cli.exec` — the allowlisted choke point for all CLI-to-router traffic.
//!
//! The Unix-Domain-Socket server (`uds.rs`) grants any same-user client access
//! to the full command router. `cli.exec` is the *only* method the external
//! `conclave-cli` binary calls; it maps a small, curated set of shell-style
//! subcommands to internal router methods and rejects everything else.
//!
//! **Security note (M5.1 review):** the match arms in [`map_argv`] are the
//! security boundary. There is intentionally no passthrough — a caller cannot
//! name an arbitrary router method through this path.

use crate::engine::{router, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct ExecReq {
    argv: Vec<String>,
}

/// Allowlisted CLI entry point — the single choke point through which the
/// external `conclave-cli` binary reaches the router (M5.1 security review).
///
/// Accepts `{ "argv": ["subcommand", …] }`, maps the argv vector to an
/// internal router method + params via [`map_argv`], then dispatches. Every
/// reachable router method is enumerated explicitly in `map_argv`; there is no
/// passthrough that allows a caller to name an arbitrary method.
///
/// # Note on `Box::pin`
/// The router contains `"cli.exec" => cli::exec(…)`, creating a potential
/// type-level recursive async cycle (`exec` → `dispatch` → `exec`). Because
/// `map_argv` never produces `"cli.exec"` as a method name, the recursion
/// never fires at runtime, but the compiler cannot verify this. `Box::pin`
/// wraps the dispatch future in a heap allocation, giving it a fixed pointer
/// size and breaking the infinite-type cycle (E0733).
pub async fn exec(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ExecReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("cli.exec: bad payload: {e}")))?;
    let (method, params) = map_argv(&req.argv)?;
    Box::pin(router::dispatch(state, method, params)).await
}

/// Pure subcommand-to-router mapping — the explicit allowlist.
///
/// Returns `(&'static str method, Value params)` for a recognised subcommand,
/// or `AppError::Invalid` for anything outside the allowlist. This function
/// has no I/O side effects; it is called synchronously from [`exec`].
///
/// # Security
/// Every `match` arm names a concrete router method literal. There is no arm
/// that passes the subcommand through as a method name. Unknown subcommands
/// (including plausible router methods like `"provider.upsert"` or
/// `"cli.exec"`) fall through to the catch-all error.
fn map_argv(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    let first = argv
        .first()
        .map(String::as_str)
        .ok_or_else(|| AppError::Invalid("cli: no subcommand given (try `help`)".into()))?;

    match first {
        // ── ws ────────────────────────────────────────────────────────────
        "ws" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                if argv.len() != 2 {
                    return Err(AppError::Invalid("cli: ws list".into()));
                }
                Ok(("workspace.list", Value::Null))
            }
            Some("use") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: ws use <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: ws use <workspaceId>".into()));
                }
                Ok(("workspace.use", json!({ "workspaceId": workspace_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: ws <list|use> — unknown ws subcommand".into(),
            )),
        },

        // ── agent ─────────────────────────────────────────────────────────
        "agent" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: agent list <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: agent list <workspaceId>".into()));
                }
                Ok(("instance.list", json!({ "workspaceId": workspace_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: agent <list> — unknown agent subcommand".into(),
            )),
        },

        // ── send ──────────────────────────────────────────────────────────
        "send" => {
            let session_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: send <sessionId> <text...>".into()))?;
            if argv.len() < 3 {
                return Err(AppError::Invalid("cli: send <sessionId> <text...>".into()));
            }
            let text = argv[2..].join(" ");
            Ok((
                "message.send",
                json!({ "sessionId": session_id, "text": text }),
            ))
        }

        // ── tell (agent→agent injection) ──────────────────────────────────
        // `tell <fromInstanceId> <toInstanceId> <text...>` → message.inject.
        // The `conclave-cli` client fills <fromInstanceId> from CONCLAVE_INSTANCE_ID
        // so a spawned agent only types `conclave tell <agentId> <text>`. Identity
        // is client-supplied; consistent with the same-user UDS trust model (a
        // local process can already reach `send`).
        "tell" => {
            let from = argv.get(1).ok_or_else(|| {
                AppError::Invalid("cli: tell <fromInstanceId> <toInstanceId> <text...>".into())
            })?;
            let to = argv.get(2).ok_or_else(|| {
                AppError::Invalid("cli: tell <fromInstanceId> <toInstanceId> <text...>".into())
            })?;
            if argv.len() < 4 {
                return Err(AppError::Invalid(
                    "cli: tell <fromInstanceId> <toInstanceId> <text...>".into(),
                ));
            }
            let text = argv[3..].join(" ");
            Ok((
                "message.inject",
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": text }),
            ))
        }

        // ── msg (read inter-agent message history) ────────────────────────
        // `list` reads the CALLER's own inbox+outbox — the client's
        // `expand_self_args` fills the instance id at argv[2] from
        // CONCLAVE_INSTANCE_ID (same idiom as `snapshot last`), so the server
        // reads it as a plain positional. `all` takes an explicit workspace id
        // and needs no self context. Both always request `withNames: true` so
        // the transcript renderer has names to show. `--limit N` is optional and
        // parsed order-independently from the tail.
        "msg" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let usage = "cli: msg list [--limit N]";
                let instance_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid(usage.into()))?;
                let (limit, rest) = take_flag(argv.get(3..).unwrap_or(&[]), "--limit");
                if !rest.is_empty() {
                    return Err(AppError::Invalid(usage.into()));
                }
                let mut params = json!({ "instanceId": instance_id, "withNames": true });
                if let Some(raw) = limit {
                    let n: i64 = raw.parse().map_err(|_| {
                        AppError::Invalid(format!("cli: msg list: --limit expects an integer, got '{raw}'"))
                    })?;
                    params["limit"] = json!(n);
                }
                Ok(("message.list", params))
            }
            Some("all") => {
                let usage = "cli: msg all <workspaceId> [--limit N]";
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid(usage.into()))?;
                let (limit, rest) = take_flag(argv.get(3..).unwrap_or(&[]), "--limit");
                if !rest.is_empty() {
                    return Err(AppError::Invalid(usage.into()));
                }
                let mut params = json!({ "workspaceId": workspace_id, "withNames": true });
                if let Some(raw) = limit {
                    let n: i64 = raw.parse().map_err(|_| {
                        AppError::Invalid(format!("cli: msg all: --limit expects an integer, got '{raw}'"))
                    })?;
                    params["limit"] = json!(n);
                }
                Ok(("message.listForWorkspace", params))
            }
            _ => Err(AppError::Invalid(
                "cli: msg <list|all> — unknown msg subcommand".into(),
            )),
        },

        // ── orient (task conclave-orient) ─────────────────────────────────
        // Wire form `orient <actorId> <workspaceId>` — the actor slot is
        // injected client-side by `expand_self_args` (self-keyed like `msg
        // list`; the packet's messages/watches/self sections need "me").
        "orient" => {
            let usage = "cli: orient <workspaceId>";
            let actor_id = argv.get(1).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let workspace_id = argv.get(2).ok_or_else(|| AppError::Invalid(usage.into()))?;
            if argv.len() != 3 {
                return Err(AppError::Invalid(usage.into()));
            }
            Ok((
                "orient.list",
                json!({ "workspaceId": workspace_id, "actorId": actor_id }),
            ))
        }

        // ── bb ────────────────────────────────────────────────────────────
        "bb" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: bb list <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: bb list <workspaceId>".into()));
                }
                Ok(("blackboard.list", json!({ "workspaceId": workspace_id })))
            }
            Some("get") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: bb get <workspaceId> <key>".into()))?;
                let key = argv
                    .get(3)
                    .ok_or_else(|| AppError::Invalid("cli: bb get <workspaceId> <key>".into()))?;
                if argv.len() != 4 {
                    return Err(AppError::Invalid("cli: bb get <workspaceId> <key>".into()));
                }
                Ok((
                    "blackboard.get",
                    json!({ "workspaceId": workspace_id, "key": key }),
                ))
            }
            Some("set") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                let key = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                let raw = argv.get(4).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                if argv.len() != 5 {
                    return Err(AppError::Invalid(
                        "cli: bb set <workspaceId> <key> <value>".into(),
                    ));
                }
                // Parse `<value>` as JSON; fall back to bare string on failure.
                let value: Value =
                    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()));
                Ok((
                    "blackboard.set",
                    json!({ "workspaceId": workspace_id, "key": key, "value": value }),
                ))
            }
            // `delete` with an `rm` alias — agents type both; one wire method.
            Some("delete") | Some("rm") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: bb delete <workspaceId> <key>".into())
                })?;
                let key = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: bb delete <workspaceId> <key>".into())
                })?;
                if argv.len() != 4 {
                    return Err(AppError::Invalid("cli: bb delete <workspaceId> <key>".into()));
                }
                Ok((
                    "blackboard.delete",
                    json!({ "workspaceId": workspace_id, "key": key }),
                ))
            }
            _ => Err(AppError::Invalid(
                "cli: bb <list|get|set|delete> — unknown bb subcommand".into(),
            )),
        },

        // ── snapshot ──────────────────────────────────────────────────────
        "snapshot" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let session_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot list <sessionId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot list <sessionId>".into()));
                }
                Ok(("snapshot.list", json!({ "sessionId": session_id })))
            }
            Some("read") => {
                let snapshot_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot read <snapshotId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot read <snapshotId>".into()));
                }
                Ok(("snapshot.read", json!({ "snapshotId": snapshot_id })))
            }
            Some("create") => {
                let session_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot create <sessionId> <type> [label]".into())
                })?;
                let snap_type = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot create <sessionId> <type> [label]".into())
                })?;
                if argv.len() > 5 {
                    return Err(AppError::Invalid(
                        "cli: snapshot create <sessionId> <type> [label]".into(),
                    ));
                }
                let mut params = json!({ "sessionId": session_id, "type": snap_type });
                if let Some(label) = argv.get(4) {
                    params["label"] = json!(label);
                }
                Ok(("snapshot.create", params))
            }
            // Agent self-handoff (strategic-compact). `save`/`last` are keyed on
            // the INSTANCE id; the `conclave-cli` client fills it from
            // CONCLAVE_INSTANCE_ID so a spawned agent types only
            // `conclave snapshot save <text>` / `conclave snapshot last`.
            Some("save") => {
                let instance_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot save <instanceId> <text...>".into())
                })?;
                if argv.len() < 4 {
                    return Err(AppError::Invalid(
                        "cli: snapshot save <instanceId> <text...>".into(),
                    ));
                }
                let text = argv[3..].join(" ");
                Ok((
                    "snapshot.save",
                    json!({ "instanceId": instance_id, "text": text }),
                ))
            }
            Some("last") => {
                let instance_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot last <instanceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot last <instanceId>".into()));
                }
                Ok(("snapshot.last", json!({ "instanceId": instance_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: snapshot <list|read|create|save|last> — unknown snapshot subcommand".into(),
            )),
        },

        // ── memory ────────────────────────────────────────────────────────
        "memory" => match argv.get(1).map(String::as_str) {
            // `memory remember <author> <workspaceId> <text...>` — the
            // `<author>` slot is filled by `conclave-cli`'s `expand_self_args`
            // from the caller's own `CONCLAVE_INSTANCE_ID` when set, or the
            // sentinel "-" outside a spawned agent (ADR 0007: we do not
            // fabricate authors — unset stays `manual`). "-" can never
            // collide with a real instance id, which is always a UUID.
            Some("remember") => {
                let author = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: memory remember <workspaceId> <text...>".into())
                })?;
                let workspace_id = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: memory remember <workspaceId> <text...>".into())
                })?;
                if argv.len() < 5 {
                    return Err(AppError::Invalid(
                        "cli: memory remember <workspaceId> <text...>".into(),
                    ));
                }
                let text = argv[4..].join(" ");
                let mut params = json!({ "workspaceId": workspace_id, "text": text });
                if author != "-" {
                    params["sourceKind"] = json!("agent");
                    params["sourceId"] = json!(author);
                }
                Ok(("memory.remember", params))
            }
            Some("search") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid(
                        "cli: memory search <workspaceId> <query...> [--limit N]".into(),
                    )
                })?;
                let mut query_words = argv.get(3..).unwrap_or(&[]);
                if query_words.is_empty() {
                    return Err(AppError::Invalid(
                        "cli: memory search <workspaceId> <query...> [--limit N]".into(),
                    ));
                }

                // A trailing `--limit N` pair is stripped from the query
                // words before joining — everything else is the query text.
                let mut limit: Option<i64> = None;
                if query_words.len() >= 2
                    && query_words[query_words.len() - 2] == "--limit"
                {
                    let raw = &query_words[query_words.len() - 1];
                    let parsed = raw.parse::<i64>().map_err(|_| {
                        AppError::Invalid(format!(
                            "cli: memory search --limit expects an integer, got '{raw}'"
                        ))
                    })?;
                    limit = Some(parsed);
                    query_words = &query_words[..query_words.len() - 2];
                }
                if query_words.is_empty() {
                    return Err(AppError::Invalid(
                        "cli: memory search <workspaceId> <query...> [--limit N]".into(),
                    ));
                }

                let query = query_words.join(" ");
                let mut params = json!({ "workspaceId": workspace_id, "query": query });
                if let Some(limit) = limit {
                    params["limit"] = json!(limit);
                }
                Ok(("memory.search", params))
            }
            Some("delete") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: memory delete <workspaceId> <chunkId>".into())
                })?;
                let id = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: memory delete <workspaceId> <chunkId>".into())
                })?;
                if argv.len() != 4 {
                    return Err(AppError::Invalid(
                        "cli: memory delete <workspaceId> <chunkId>".into(),
                    ));
                }
                Ok((
                    "memory.delete",
                    json!({ "workspaceId": workspace_id, "id": id }),
                ))
            }
            Some("status") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: memory status <workspaceId>".into())
                })?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: memory status <workspaceId>".into()));
                }
                Ok(("memory.status", json!({ "workspaceId": workspace_id })))
            }
            // `memory propose <proposer> <workspaceId> <text...> [--source-note NOTE]`
            // — the `<proposer>` slot is filled by `conclave-cli`'s
            // `expand_self_args` from `CONCLAVE_INSTANCE_ID`. A trailing
            // `--source-note NOTE` pair is stripped before the text is joined.
            Some("propose") => {
                let usage = || {
                    AppError::Invalid(
                        "cli: memory propose <workspaceId> <text...> [--source-note NOTE]".into(),
                    )
                };
                let proposer = argv.get(2).ok_or_else(usage)?;
                let workspace_id = argv.get(3).ok_or_else(usage)?;
                let mut rest = argv.get(4..).unwrap_or(&[]);
                let mut source_note: Option<&str> = None;
                if rest.len() >= 2 && rest[rest.len() - 2] == "--source-note" {
                    source_note = Some(&rest[rest.len() - 1]);
                    rest = &rest[..rest.len() - 2];
                }
                if rest.is_empty() {
                    return Err(usage());
                }
                let text = rest.join(" ");
                let mut params =
                    json!({ "workspaceId": workspace_id, "proposerId": proposer, "text": text });
                if let Some(note) = source_note {
                    params["sourceNote"] = json!(note);
                }
                Ok(("memory.propose", params))
            }
            Some("queue") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid(
                        "cli: memory queue <workspaceId> [--state pending|approved|rejected]".into(),
                    )
                })?;
                let mut params = json!({ "workspaceId": workspace_id });
                match argv.get(3..).unwrap_or(&[]) {
                    [] => {}
                    [flag, state] if flag == "--state" => {
                        params["state"] = json!(state);
                    }
                    _ => {
                        return Err(AppError::Invalid(
                            "cli: memory queue <workspaceId> [--state pending|approved|rejected]"
                                .into(),
                        ))
                    }
                }
                Ok(("memory.queue", params))
            }
            // `memory approve|reject <reviewer> <workspaceId> <proposalId> [--reason TEXT...]`
            // — `<reviewer>` is filled by `expand_self_args` from
            // `CONCLAVE_INSTANCE_ID`. An optional leading `--reason` consumes the
            // rest as free-text prose.
            Some(verb @ ("approve" | "reject")) => {
                let method = if verb == "approve" {
                    "memory.approve"
                } else {
                    "memory.reject"
                };
                let usage = || {
                    AppError::Invalid(format!(
                        "cli: memory {verb} <workspaceId> <proposalId> [--reason TEXT...]"
                    ))
                };
                let reviewer = argv.get(2).ok_or_else(usage)?;
                let workspace_id = argv.get(3).ok_or_else(usage)?;
                let proposal_id = argv.get(4).ok_or_else(usage)?;
                let mut params = json!({
                    "workspaceId": workspace_id,
                    "reviewerId": reviewer,
                    "proposalId": proposal_id,
                });
                match argv.get(5..).unwrap_or(&[]) {
                    [] => {}
                    [flag, reason @ ..] if flag == "--reason" && !reason.is_empty() => {
                        params["reason"] = json!(reason.join(" "));
                    }
                    _ => return Err(usage()),
                }
                Ok((method, params))
            }
            _ => Err(AppError::Invalid(
                "cli: memory <remember|search|delete|status|propose|queue|approve|reject> — \
                 unknown memory subcommand"
                    .into(),
            )),
        },

        // ── artifact (plan design-artifact-store) ────────────────────────
        // `add` arrives ALREADY expanded by `conclave-cli`: the creator-agent
        // slot right after `add` is filled from `CONCLAVE_INSTANCE_ID` (or the
        // sentinel "-" outside a spawned agent, same idiom as `memory
        // remember`), and any `--file <path>` was resolved to `--content
        // <text>` client-side (map_argv does no file I/O). `list`/`get` are
        // read-only and pass through unexpanded.
        "artifact" => match argv.get(1).map(String::as_str) {
            Some("add") => {
                let usage = || {
                    AppError::Invalid(
                        "cli: artifact add <workspaceId> --title <t> --kind <k> (--file <path> | --content <text>)".into(),
                    )
                };
                let agent = argv.get(2).ok_or_else(usage)?;
                let workspace_id = argv.get(3).ok_or_else(usage)?;

                // Parse the flags sequentially — this is the security choke
                // point (M5.1: no passthrough), so it validates strictly rather
                // than picking the first match and ignoring the rest. Each of
                // --title/--kind/--content/--filename may appear at most once;
                // --file is rejected (the client resolves it to --content —
                // the server never reads files); unknown flags, duplicates, and
                // dangling values are all errors.
                let mut title: Option<&str> = None;
                let mut kind: Option<&str> = None;
                let mut content: Option<&str> = None;
                let mut filename: Option<&str> = None;
                let flags = argv.get(4..).unwrap_or(&[]);
                let mut i = 0;
                while i < flags.len() {
                    let name = flags[i].as_str();
                    let slot: &mut Option<&str> = match name {
                        "--title" => &mut title,
                        "--kind" => &mut kind,
                        "--content" => &mut content,
                        "--filename" => &mut filename,
                        "--file" => {
                            return Err(AppError::Invalid(
                                "cli: artifact add: --file must be resolved to --content by the client; the server does not read files".into(),
                            ))
                        }
                        other => {
                            return Err(AppError::Invalid(format!(
                                "cli: artifact add: unknown flag '{other}'"
                            )))
                        }
                    };
                    if slot.is_some() {
                        return Err(AppError::Invalid(format!(
                            "cli: artifact add: duplicate flag '{name}'"
                        )));
                    }
                    let value = flags.get(i + 1).ok_or_else(|| {
                        AppError::Invalid(format!("cli: artifact add: flag '{name}' expects a value"))
                    })?;
                    *slot = Some(value.as_str());
                    i += 2;
                }

                let title = title
                    .ok_or_else(|| AppError::Invalid("cli: artifact add requires --title <t>".into()))?;
                let kind = kind
                    .ok_or_else(|| AppError::Invalid("cli: artifact add requires --kind <k>".into()))?;
                let content = content.ok_or_else(|| {
                    AppError::Invalid(
                        "cli: artifact add requires --content <text> or --file <path>".into(),
                    )
                })?;
                let mut params =
                    json!({ "workspaceId": workspace_id, "title": title, "kind": kind, "content": content });
                if agent != "-" {
                    params["agentId"] = json!(agent);
                }
                if let Some(f) = filename {
                    params["filename"] = json!(f);
                }
                Ok(("artifact.add", params))
            }
            Some("list") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: artifact list <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: artifact list <workspaceId>".into()));
                }
                Ok(("artifact.list", json!({ "workspaceId": workspace_id })))
            }
            Some("get") => {
                let id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: artifact get <id>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: artifact get <id>".into()));
                }
                Ok(("artifact.get", json!({ "id": id })))
            }
            _ => Err(AppError::Invalid(
                "cli: artifact <add|list|get> — unknown artifact subcommand".into(),
            )),
        },

        // ── position / org (spec position-system §5) ─────────────────────
        // `position set <ws> <agentId> [--level v|none] [--supervisor v|none]`
        // → instance.setPosition. Strict sequential flag parse (0b744976
        // precedent): each flag at most once, unknown/dangling rejected, at
        // least one flag required. `<ws>` is a positional for grammar
        // consistency but is NOT forwarded — the spec §5.1 payload keys on the
        // globally-unique workspaceAgentId. A flag value of "none" clears
        // (JSON null); an absent flag omits the key entirely (tri-state
        // absent=keep). `org <ws>` reuses instance.list (Q5: the tree is
        // derived client-side; no new read command).
        "position" => match argv.get(1).map(String::as_str) {
            Some("set") => {
                let usage = || {
                    AppError::Invalid(
                        "cli: position set <ws> <agentId> [--level <l>|none] [--supervisor <agentId>|none]".into(),
                    )
                };
                let ws = argv.get(2).ok_or_else(usage)?;
                let agent_id = argv.get(3).ok_or_else(usage)?;
                let mut level: Option<&str> = None;
                let mut supervisor: Option<&str> = None;
                let flags = argv.get(4..).unwrap_or(&[]);
                let mut i = 0;
                while i < flags.len() {
                    let name = flags[i].as_str();
                    let slot: &mut Option<&str> = match name {
                        "--level" => &mut level,
                        "--supervisor" => &mut supervisor,
                        other => {
                            return Err(AppError::Invalid(format!(
                                "cli: position set: unknown flag '{other}'"
                            )))
                        }
                    };
                    if slot.is_some() {
                        return Err(AppError::Invalid(format!(
                            "cli: position set: duplicate flag '{name}'"
                        )));
                    }
                    let value = flags.get(i + 1).ok_or_else(|| {
                        AppError::Invalid(format!("cli: position set: flag '{name}' expects a value"))
                    })?;
                    *slot = Some(value.as_str());
                    i += 2;
                }
                if level.is_none() && supervisor.is_none() {
                    return Err(AppError::Invalid(
                        "cli: position set requires at least one of --level or --supervisor".into(),
                    ));
                }
                // Forward <ws> so the command can reject a wrong-workspace
                // target (646ec73a) — the agent id is globally unique, so an
                // unenforced <ws> would silently mutate an agent elsewhere.
                let mut params = json!({ "workspaceId": ws, "workspaceAgentId": agent_id });
                if let Some(l) = level {
                    params["level"] = if l == "none" { Value::Null } else { json!(l) };
                }
                if let Some(s) = supervisor {
                    params["supervisorAgentId"] = if s == "none" { Value::Null } else { json!(s) };
                }
                Ok(("instance.setPosition", params))
            }
            _ => Err(AppError::Invalid(
                "cli: position <set> — unknown position subcommand".into(),
            )),
        },

        "org" => {
            let workspace_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: org <workspaceId>".into()))?;
            if argv.len() != 2 {
                return Err(AppError::Invalid("cli: org <workspaceId>".into()));
            }
            Ok(("instance.list", json!({ "workspaceId": workspace_id })))
        }

        // ── task (ADR 0008) ──────────────────────────────────────────────
        // Every self-keyed verb (all but `list`/`get`/`create`) arrives here
        // ALREADY expanded by `conclave-cli`'s `expand_self_args` — the
        // actorId slot right after the subcommand name is filled from
        // `CONCLAVE_INSTANCE_ID`, same idiom as `tell`/`restart`. `gate` also
        // arrives pre-computed (cmd already RAN client-side — see the risk
        // ledger note: gates never run engine-side).
        "task" => map_task_argv(argv),

        // ── run ───────────────────────────────────────────────────────────
        "run" => {
            let orchestrator_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: run <orchestratorId> <prompt...>".into()))?;
            if argv.len() < 3 {
                return Err(AppError::Invalid(
                    "cli: run <orchestratorId> <prompt...>".into(),
                ));
            }
            let prompt = argv[2..].join(" ");
            Ok((
                "fusion.run",
                json!({ "orchestratorId": orchestrator_id, "prompt": prompt }),
            ))
        }

        // ── restart (ADR 0006: self-triggered restart) ────────────────────
        // Always self-targeting — `conclave-cli`'s `expand_self_args` fills
        // the instance id from `CONCLAVE_INSTANCE_ID` before this ever runs,
        // so a bare `conclave restart` (agent-typed) reaches here as
        // `["restart", <selfId>]`.
        "restart" => {
            let instance_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: restart <instanceId>".into()))?;
            if argv.len() != 2 {
                return Err(AppError::Invalid("cli: restart <instanceId>".into()));
            }
            Ok((
                "instance.restart",
                json!({ "workspaceAgentId": instance_id, "self": true }),
            ))
        }

        // ── design (Lane R: deterministic design-review gate) ─────────────
        // `design review <workspaceId> [--json]` → design.review. The PLAIN
        // form is the GATE — it exits non-zero on serious findings, so it drops
        // into `conclave task gate` / `… && next`. `--json` is DATA retrieval —
        // it always exits 0 and returns the full `{ pass, findings, assertions }`
        // report (gate with the plain form; see the design.review handler).
        "design" => match argv.get(1).map(String::as_str) {
            Some("review") => {
                let usage =
                    || AppError::Invalid("cli: design review <workspaceId> [--json]".into());
                let ws = argv.get(2).ok_or_else(usage)?;
                let json = match argv.get(3).map(String::as_str) {
                    None => false,
                    Some("--json") => true,
                    Some(_) => return Err(usage()),
                };
                if argv.len() > 4 {
                    return Err(usage());
                }
                Ok(("design.review", json!({ "workspaceId": ws, "json": json })))
            }
            _ => Err(AppError::Invalid(
                "cli: design review <workspaceId> [--json] — unknown design subcommand".into(),
            )),
        },

        // ── browser (in-app browser agent tools) ──────────────────────────
        // `browser <open|goto|status|snapshot|click|type|eval|close>`. Agent
        // verbs auto-scope to the caller's OWN tab: `conclave-cli` injects the
        // server-trusted caller id (from CONCLAVE_INSTANCE_ID) POSITIONALLY at
        // argv[2] — right after the verb, the same idiom as `task claim` — so
        // free-text verb args (eval/type) can NEVER be mistaken for it
        // (anti-spoof). We splice that id out here so the per-verb parsing in
        // `map_browser_verb` sees the ORIGINAL positional shape, then re-attach
        // it as the trusted `callerId`. `status` is a global read (no id).
        "browser" => {
            let verb = argv.get(1).map(String::as_str);
            let (caller_id, cleaned): (Option<String>, Vec<String>) = if verb == Some("status") {
                (None, argv.to_vec())
            } else {
                let cid = argv.get(2).cloned();
                let mut cleaned = Vec::with_capacity(argv.len().saturating_sub(1));
                cleaned.push(argv[0].clone()); // "browser"
                if let Some(v) = argv.get(1) {
                    cleaned.push(v.clone()); // verb
                }
                // Skip argv[2] (the injected caller id); keep the verb's own args.
                cleaned.extend_from_slice(argv.get(3..).unwrap_or(&[]));
                (cid, cleaned)
            };
            let (method, mut params) = map_browser_verb(&cleaned)?;
            if let Some(cid) = caller_id.filter(|s| !s.is_empty()) {
                match &mut params {
                    Value::Object(map) => {
                        map.insert("callerId".to_string(), Value::String(cid));
                    }
                    Value::Null => params = json!({ "callerId": cid }),
                    _ => {}
                }
            }
            Ok((method, params))
        }

        // ── code (tree-sitter code intelligence) ──────────────────────────
        "code" => map_code_argv(argv),

        // ── proxy (context optimizer) ─────────────────────────────────────
        "proxy" => map_proxy_argv(argv),

        // ── unknown — security catch-all ──────────────────────────────────
        other => Err(AppError::Invalid(format!(
            "cli: unknown subcommand '{other}' (allowed: ws, agent, send, tell, msg, bb, snapshot, memory, task, run, restart, design, browser, code, proxy)"
        ))),
    }
}

/// Map a `browser <verb> [args…]` argv (with the injected caller id already
/// spliced out by the caller) to its router method + params. URL scheme
/// normalization happens server-side in `runtime::browser`; this only maps
/// verbs → router commands + payloads. `argv[0]` is `"browser"`, `argv[1]` the
/// verb, `argv[2..]` the verb's own args — the exact positional shape the human
/// types, since the caller id (if any) was removed upstream.
fn map_browser_verb(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    match argv.get(1).map(String::as_str) {
        Some("open") => {
            if argv.len() != 3 {
                return Err(AppError::Invalid("cli: browser open <url>".into()));
            }
            Ok(("browser.open", json!({ "url": argv[2] })))
        }
        Some("goto") => {
            if argv.len() != 3 {
                return Err(AppError::Invalid("cli: browser goto <url>".into()));
            }
            Ok(("browser.goto", json!({ "url": argv[2] })))
        }
        Some("status") => {
            if argv.len() != 2 {
                return Err(AppError::Invalid("cli: browser status".into()));
            }
            Ok(("browser.status", Value::Null))
        }
        Some("snapshot") => {
            let (max, rest) = take_flag(argv.get(2..).unwrap_or(&[]), "--max-text");
            if !rest.is_empty() {
                return Err(AppError::Invalid(
                    "cli: browser snapshot [--max-text N]".into(),
                ));
            }
            let mut params = json!({});
            if let Some(raw) = max {
                let n = raw.parse::<i64>().map_err(|_| {
                    AppError::Invalid("cli: browser snapshot: --max-text expects an integer".into())
                })?;
                params["maxText"] = json!(n);
            }
            Ok(("browser.snapshot", params))
        }
        Some("click") => {
            if argv.len() != 3 {
                return Err(AppError::Invalid("cli: browser click <selector>".into()));
            }
            Ok(("browser.click", json!({ "selector": argv[2] })))
        }
        Some("type") => {
            let selector = argv.get(2).ok_or_else(|| {
                AppError::Invalid("cli: browser type <selector> <text...>".into())
            })?;
            if argv.len() < 4 {
                return Err(AppError::Invalid(
                    "cli: browser type <selector> <text...>".into(),
                ));
            }
            let text = argv[3..].join(" ");
            Ok(("browser.type", json!({ "selector": selector, "text": text })))
        }
        Some("eval") => {
            if argv.len() < 3 {
                return Err(AppError::Invalid("cli: browser eval <js...>".into()));
            }
            Ok(("browser.eval", json!({ "js": argv[2..].join(" ") })))
        }
        Some("close") => {
            if argv.len() != 2 {
                return Err(AppError::Invalid("cli: browser close".into()));
            }
            Ok(("browser.close", Value::Null))
        }
        Some("screenshot") => {
            // path is positional (already absolute — the CLI resolved it in the
            // agent's cwd); --width/--height are optional f64 flags.
            let after = argv.get(2..).unwrap_or(&[]);
            let (width_raw, after) = take_flag(after, "--width");
            let (height_raw, after) = take_flag(&after, "--height");
            let path = after.first().cloned().ok_or_else(|| {
                AppError::Invalid("cli: browser screenshot <path> [--width N] [--height N]".into())
            })?;
            if after.len() != 1 {
                return Err(AppError::Invalid(
                    "cli: browser screenshot <path> [--width N] [--height N]".into(),
                ));
            }
            let mut params = json!({ "path": path });
            if let Some(raw) = width_raw {
                let n = raw.parse::<f64>().map_err(|_| {
                    AppError::Invalid("cli: browser screenshot: --width expects a number".into())
                })?;
                params["width"] = json!(n);
            }
            if let Some(raw) = height_raw {
                let n = raw.parse::<f64>().map_err(|_| {
                    AppError::Invalid("cli: browser screenshot: --height expects a number".into())
                })?;
                params["height"] = json!(n);
            }
            Ok(("browser.screenshot", params))
        }
        _ => Err(AppError::Invalid(
            "cli: browser <open|goto|status|snapshot|screenshot|click|type|eval|close> — unknown browser subcommand".into(),
        )),
    }
}

/// Pull a single `--flag value` pair out of `words`, wherever it appears
/// (order-independent — every `task` flag-bearing verb accepts its flags in
/// any order). Returns `(value, remaining words with the pair removed)`, or
/// `(None, words unchanged)` if the flag is absent.
fn take_flag(words: &[String], flag: &str) -> (Option<String>, Vec<String>) {
    if let Some(pos) = words.iter().position(|w| w == flag) {
        if let Some(value) = words.get(pos + 1) {
            let mut rest = words.to_vec();
            rest.drain(pos..=pos + 1);
            return (Some(value.clone()), rest);
        }
    }
    (None, words.to_vec())
}

/// Pull a standalone `--flag` switch out of `words`, wherever it appears
/// (order-independent like [`take_flag`]). Returns `(present, remaining
/// words)` or `(false, words unchanged)` if the switch is absent.
fn take_switch(words: &[String], flag: &str) -> (bool, Vec<String>) {
    if let Some(pos) = words.iter().position(|w| w == flag) {
        let mut rest = words.to_vec();
        rest.remove(pos);
        return (true, rest);
    }
    (false, words.to_vec())
}

fn map_proxy_argv(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    let usage =
        "cli: proxy <status|mode <off|log|rewrite>|threshold <ratio>|report [--since-hours N]>";
    match argv.get(1).map(String::as_str) {
        Some("status") if argv.len() == 2 => Ok(("proxy.status", Value::Null)),
        Some("mode") if argv.len() == 3 => match argv[2].as_str() {
            "off" | "log" | "rewrite" => Ok(("proxy.mode", json!({ "mode": argv[2] }))),
            _ => Err(AppError::Invalid(usage.into())),
        },
        Some("threshold") if argv.len() == 3 => {
            let ratio = argv[2]
                .parse::<f32>()
                .map_err(|_| AppError::Invalid(usage.into()))?;
            Ok(("proxy.threshold", json!({ "ratio": ratio })))
        }
        Some("report") => {
            let (since_hours, rest) = take_flag(&argv[2..], "--since-hours");
            if !rest.is_empty() {
                return Err(AppError::Invalid(usage.into()));
            }
            let mut params = json!({});
            if let Some(raw) = since_hours {
                let value = raw.parse::<i64>().map_err(|_| {
                    AppError::Invalid(
                        "cli: proxy report: --since-hours expects a non-negative integer".into(),
                    )
                })?;
                if value < 0 {
                    return Err(AppError::Invalid(
                        "cli: proxy report: --since-hours expects a non-negative integer".into(),
                    ));
                }
                params["sinceHours"] = json!(value);
            }
            Ok(("proxy.report", params))
        }
        _ => Err(AppError::Invalid(usage.into())),
    }
}

const CODE_USAGE: &str =
    "cli: code <stats|files|tree|symbols|find|callers|callees|refs|impact|rename|rewrite> …";

fn code_usage() -> AppError {
    AppError::Invalid(CODE_USAGE.into())
}

fn code_path(words: &[String]) -> Result<(String, Vec<String>), AppError> {
    let (_, words) = take_switch(words, "--json");
    let (path, rest) = take_flag(&words, "--path");
    match path {
        Some(path) if !path.starts_with("--") => Ok((path, rest)),
        _ => Err(code_usage()),
    }
}

fn code_number(raw: String, flag: &str) -> Result<i64, AppError> {
    raw.parse::<i64>()
        .map_err(|_| AppError::Invalid(format!("cli: code: {flag} expects a non-negative integer")))
        .and_then(|value| {
            if value < 0 {
                Err(AppError::Invalid(format!(
                    "cli: code: {flag} expects a non-negative integer"
                )))
            } else {
                Ok(value)
            }
        })
}

/// `code <verb> …` allowlist. `run_code` has already resolved and injected an
/// absolute `--path`; this layer only validates argv and maps it to camelCase
/// router params. `--json` is accepted for compatibility and ignored because
/// every code response is JSON.
fn map_code_argv(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    let verb = argv.get(1).map(String::as_str);
    let (path, words) = code_path(argv.get(2..).unwrap_or(&[]))?;

    match verb {
        Some("stats") | Some("tree") => {
            if !words.is_empty() {
                return Err(code_usage());
            }
            let method = if verb == Some("stats") {
                "code.stats"
            } else {
                "code.tree"
            };
            Ok((method, json!({ "path": path })))
        }
        Some("files") => {
            let (limit, rest) = take_flag(&words, "--limit");
            if !rest.is_empty() {
                return Err(code_usage());
            }
            let mut params = json!({ "path": path });
            if let Some(raw) = limit {
                params["limit"] = json!(code_number(raw, "--limit")?);
            }
            Ok(("code.files", params))
        }
        Some("symbols") => {
            let (all, words) = take_switch(&words, "--all");
            let (kind, words) = take_flag(&words, "--kind");
            let (limit, rest) = take_flag(&words, "--limit");
            if rest.len() > 1 || rest.first().is_some_and(|word| word.starts_with("--")) {
                return Err(code_usage());
            }
            let mut params = json!({ "path": path });
            if let Some(target) = rest.first() {
                params["target"] = json!(target);
            }
            if all {
                params["all"] = json!(true);
            }
            if let Some(raw) = kind {
                let kinds: Vec<&str> = raw.split(',').filter(|kind| !kind.is_empty()).collect();
                if kinds.is_empty() {
                    return Err(code_usage());
                }
                params["kind"] = json!(kinds);
            }
            if let Some(raw) = limit {
                params["limit"] = json!(code_number(raw, "--limit")?);
            }
            Ok(("code.symbols", params))
        }
        Some("find") => {
            let (exact, words) = take_switch(&words, "--exact");
            let (limit, rest) = take_flag(&words, "--limit");
            if rest.len() != 1 || rest[0].starts_with("--") {
                return Err(code_usage());
            }
            let mut params = json!({ "path": path, "name": rest[0] });
            if exact {
                params["exact"] = json!(true);
            }
            if let Some(raw) = limit {
                params["limit"] = json!(code_number(raw, "--limit")?);
            }
            Ok(("code.find", params))
        }
        Some("callers") | Some("callees") => {
            let (depth, rest) = take_flag(&words, "--depth");
            if rest.len() != 1 || rest[0].starts_with("--") {
                return Err(code_usage());
            }
            let mut params = json!({ "path": path, "name": rest[0] });
            if let Some(raw) = depth {
                params["depth"] = json!(code_number(raw, "--depth")?);
            }
            let method = if verb == Some("callers") {
                "code.callers"
            } else {
                "code.callees"
            };
            Ok((method, params))
        }
        Some("refs") => {
            let (limit, rest) = take_flag(&words, "--limit");
            if rest.len() != 1 || rest[0].starts_with("--") {
                return Err(code_usage());
            }
            let mut params = json!({ "path": path, "name": rest[0] });
            if let Some(raw) = limit {
                params["limit"] = json!(code_number(raw, "--limit")?);
            }
            Ok(("code.refs", params))
        }
        Some("impact") => {
            if words.len() != 1 || words[0].starts_with("--") {
                return Err(code_usage());
            }
            Ok(("code.impact", json!({ "path": path, "name": words[0] })))
        }
        Some("rename") => {
            let (apply, words) = take_switch(&words, "--apply");
            let (lang, words) = take_flag(&words, "--lang");
            let (anchor, rest) = take_flag(&words, "--anchor");
            if rest.len() != 2 || rest.iter().any(|word| word.starts_with("--")) {
                return Err(code_usage());
            }
            let mut params = json!({
                "path": path,
                "old": rest[0],
                "new": rest[1],
            });
            if apply {
                params["apply"] = json!(true);
            }
            if let Some(lang) = lang {
                params["lang"] = json!(lang);
            }
            if let Some(anchor) = anchor {
                params["anchor"] = json!(anchor);
            }
            Ok(("code.rename", params))
        }
        Some("rewrite") => {
            let (pattern, words) = take_flag(&words, "--pattern");
            let (rewrite, words) = take_flag(&words, "--rewrite");
            let (apply, words) = take_switch(&words, "--apply");
            let (lang, rest) = take_flag(&words, "--lang");
            if !rest.is_empty() {
                return Err(code_usage());
            }
            let pattern = pattern.filter(|value| !value.starts_with("--"));
            let rewrite = rewrite.filter(|value| !value.starts_with("--"));
            let (Some(pattern), Some(rewrite)) = (pattern, rewrite) else {
                return Err(code_usage());
            };
            let mut params = json!({
                "path": path,
                "pattern": pattern,
                "rewrite": rewrite,
            });
            if apply {
                params["apply"] = json!(true);
            }
            if let Some(lang) = lang {
                params["lang"] = json!(lang);
            }
            Ok(("code.rewrite", params))
        }
        _ => Err(code_usage()),
    }
}

/// `task <verb> …` — the ADR 0008 allowlist. Self-keyed verbs (everything but
/// `list`/`get`/`create`) arrive pre-expanded by `conclave-cli` with the
/// actorId slot filled right after the verb name.
fn map_task_argv(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    let verb = argv.get(1).map(String::as_str);
    match verb {
        Some("list") => {
            let usage = "cli: task list <workspaceId> [--state s] [--full | --all]";
            let workspace_id = argv
                .get(2)
                .ok_or_else(|| AppError::Invalid(usage.into()))?;
            let (state, rest) = take_flag(&argv[3..], "--state");
            let (full, rest) = take_switch(&rest, "--full");
            let (all, rest) = take_switch(&rest, "--all");
            // `--full` already includes every state — combining it with
            // `--all` is contradictory input, rejected like any stray word.
            if !rest.is_empty() || (full && all) {
                return Err(AppError::Invalid(usage.into()));
            }
            let mut params = json!({ "workspaceId": workspace_id });
            if let Some(state) = state {
                params["state"] = json!(state);
            }
            // Ruling 3f1ab20e (task cli-output-economy): the CLI board is slim
            // by default; `--full` restores the pre-slim full shape with the
            // plan included (subsumes the old `--full` = includePlan meaning);
            // `--all` keeps the slim shape but includes merged/abandoned.
            if full {
                params["slim"] = json!(false);
                params["includePlan"] = json!(true);
            } else {
                params["slim"] = json!(true);
                if all {
                    params["all"] = json!(true);
                }
            }
            Ok(("task.list", params))
        }

        Some("get") => {
            let workspace_id = argv
                .get(2)
                .ok_or_else(|| AppError::Invalid("cli: task get <workspaceId> <slug>".into()))?;
            let slug = argv
                .get(3)
                .ok_or_else(|| AppError::Invalid("cli: task get <workspaceId> <slug>".into()))?;
            if argv.len() != 4 {
                return Err(AppError::Invalid("cli: task get <workspaceId> <slug>".into()));
            }
            Ok((
                "task.get",
                json!({ "workspaceId": workspace_id, "slug": slug }),
            ))
        }

        Some("brief") => {
            let workspace_id = argv
                .get(2)
                .ok_or_else(|| {
                    AppError::Invalid("cli: task brief <workspaceId> <slug> [--limit N]".into())
                })?;
            let slug = argv
                .get(3)
                .ok_or_else(|| {
                    AppError::Invalid("cli: task brief <workspaceId> <slug> [--limit N]".into())
                })?;
            let (limit, rest) = take_flag(&argv[4..], "--limit");
            if !rest.is_empty() {
                return Err(AppError::Invalid(
                    "cli: task brief <workspaceId> <slug> [--limit N]".into(),
                ));
            }
            let mut params = json!({ "workspaceId": workspace_id, "slug": slug });
            if let Some(raw) = limit {
                let n: usize = raw.parse().map_err(|_| {
                    AppError::Invalid(format!(
                        "cli: task brief: --limit expects an integer, got '{raw}'"
                    ))
                })?;
                params["limit"] = json!(n);
            }
            Ok(("task.brief", params))
        }

        // `task create <ws> <slug> <title...> [--boundary p1,p2] [--canon txt]
        //  [--owner id] [--plan text]` — `conclave-cli` resolves `--plan-file`
        // to `--plan <contents>` and defaults `--owner` to the caller's own
        // instance id before this ever runs (owner is optional enrichment,
        // same "-" sentinel philosophy as `memory remember`, except here a
        // missing owner is simply absent rather than stamped).
        Some("create") => {
            let workspace_id = argv.get(2).ok_or_else(|| {
                AppError::Invalid(
                    "cli: task create <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--owner id] [--plan text] [--watchers id,id]".into(),
                )
            })?;
            let slug = argv.get(3).ok_or_else(|| {
                AppError::Invalid(
                    "cli: task create <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--owner id] [--plan text] [--watchers id,id]".into(),
                )
            })?;
            let rest = argv.get(4..).unwrap_or(&[]).to_vec();
            let (boundary, rest) = take_flag(&rest, "--boundary");
            let (canon, rest) = take_flag(&rest, "--canon");
            let (owner, rest) = take_flag(&rest, "--owner");
            let (plan, rest) = take_flag(&rest, "--plan");
            let (watchers, rest) = take_flag(&rest, "--watchers");
            if rest.is_empty() {
                return Err(AppError::Invalid(
                    "cli: task create <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--owner id] [--plan text] [--watchers id,id]".into(),
                ));
            }
            let title = rest.join(" ");
            let mut params = json!({ "workspaceId": workspace_id, "slug": slug, "title": title });
            if let Some(boundary) = boundary {
                let paths: Vec<&str> = boundary.split(',').filter(|s| !s.is_empty()).collect();
                params["fileBoundary"] = json!(paths);
            }
            if let Some(canon) = canon {
                params["designCanon"] = json!(canon);
            }
            if let Some(owner) = owner {
                params["ownerAgentId"] = json!(owner);
            }
            if let Some(plan) = plan {
                params["plan"] = json!(plan);
            }
            if let Some(watchers) = watchers {
                // Same comma-split-and-drop-empties shape as `--boundary`; the
                // engine caps and deduplicates before touching `task_watch`.
                let ids: Vec<&str> = watchers.split(',').filter(|s| !s.is_empty()).collect();
                params["watcherAgentIds"] = json!(ids);
            }
            Ok(("task.create", params))
        }

        Some("claim") => {
            let (actor_id, workspace_id, slug) = task_actor_ws_slug(argv, "task claim")?;
            Ok((
                "task.claim",
                json!({ "workspaceId": workspace_id, "slug": slug, "actorId": actor_id }),
            ))
        }

        // Pre-computed by `conclave-cli` like `gate` (the checkout reads —
        // plan bytes, SHA-256, consumed files, git ancestry — already RAN
        // client-side; the engine has no checkout to read):
        // `task plan-check <actorId> <ws> <slug> <planPath> <fingerprint>
        //  <ancestor 0|1> <planContent> <filesJson>`.
        Some("plan-check") => {
            let usage = "cli: task plan-check <workspaceId> <slug>";
            let actor_id = argv.get(2).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let workspace_id = argv.get(3).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let slug = argv.get(4).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let plan_path = argv.get(5).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let fingerprint = argv.get(6).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let ancestor_raw = argv.get(7).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let plan_content = argv.get(8).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let files_raw = argv.get(9).ok_or_else(|| AppError::Invalid(usage.into()))?;
            if argv.len() != 10 {
                return Err(AppError::Invalid(usage.into()));
            }
            let ancestor = match ancestor_raw.as_str() {
                "1" => true,
                "0" => false,
                other => {
                    return Err(AppError::Invalid(format!(
                        "cli: task plan-check: bad ancestor flag '{other}' (expected 0 or 1)"
                    )))
                }
            };
            let files: Value = serde_json::from_str(files_raw).map_err(|e| {
                AppError::Invalid(format!("cli: task plan-check: bad files JSON: {e}"))
            })?;
            if !files.is_object() {
                return Err(AppError::Invalid(
                    "cli: task plan-check: files must be a JSON object of path -> content".into(),
                ));
            }
            Ok((
                "task.planCheck",
                json!({
                    "workspaceId": workspace_id, "slug": slug, "actorId": actor_id,
                    "planPath": plan_path, "planFingerprint": fingerprint,
                    "baseShaAncestor": ancestor, "planContent": plan_content,
                    "files": files,
                }),
            ))
        }

        Some("state") => {
            let actor_id = argv
                .get(2)
                .ok_or_else(|| AppError::Invalid("cli: task state <workspaceId> <slug> <state>".into()))?;
            let workspace_id = argv
                .get(3)
                .ok_or_else(|| AppError::Invalid("cli: task state <workspaceId> <slug> <state>".into()))?;
            let slug = argv
                .get(4)
                .ok_or_else(|| AppError::Invalid("cli: task state <workspaceId> <slug> <state>".into()))?;
            let new_state = argv
                .get(5)
                .ok_or_else(|| AppError::Invalid("cli: task state <workspaceId> <slug> <state>".into()))?;
            if argv.len() != 6 {
                return Err(AppError::Invalid(
                    "cli: task state <workspaceId> <slug> <state>".into(),
                ));
            }
            Ok((
                "task.setState",
                json!({
                    "workspaceId": workspace_id, "slug": slug,
                    "state": new_state, "actorId": actor_id
                }),
            ))
        }

        Some("note") => {
            let actor_id = argv
                .get(2)
                .ok_or_else(|| AppError::Invalid("cli: task note <workspaceId> <slug> <text...>".into()))?;
            let workspace_id = argv
                .get(3)
                .ok_or_else(|| AppError::Invalid("cli: task note <workspaceId> <slug> <text...>".into()))?;
            let slug = argv
                .get(4)
                .ok_or_else(|| AppError::Invalid("cli: task note <workspaceId> <slug> <text...>".into()))?;
            if argv.len() < 6 {
                return Err(AppError::Invalid(
                    "cli: task note <workspaceId> <slug> <text...>".into(),
                ));
            }
            let text = argv[5..].join(" ");
            Ok((
                "task.note",
                json!({
                    "workspaceId": workspace_id, "slug": slug,
                    "actorId": actor_id, "text": text
                }),
            ))
        }

        // Pre-computed by `conclave-cli` (the shell command already RAN
        // client-side — never engine-side, ADR 0008 risk ledger):
        // `task gate <actorId> <ws> <slug> <cmd> <exit> <sha> <cwd> <tail>`.
        Some("gate") => {
            let usage = "cli: task gate <workspaceId> <slug> -- <cmd...>";
            let actor_id = argv.get(2).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let workspace_id = argv.get(3).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let slug = argv.get(4).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let cmd = argv.get(5).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let exit_raw = argv.get(6).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let sha = argv.get(7).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let cwd = argv.get(8).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let tail = argv.get(9).ok_or_else(|| AppError::Invalid(usage.into()))?;
            // Word 11 (optional): the client-side path of the FULL gate log
            // (task cli-output-economy). Absent from pre-logPath CLI builds,
            // whose 10-word form must keep recording unchanged.
            if argv.len() != 10 && argv.len() != 11 {
                return Err(AppError::Invalid(usage.into()));
            }
            let exit: i64 = exit_raw
                .parse()
                .map_err(|_| AppError::Invalid(format!("cli: task gate: bad exit code '{exit_raw}'")))?;
            let mut params = json!({
                "workspaceId": workspace_id, "slug": slug, "actorId": actor_id,
                "cmd": cmd, "exit": exit, "sha": sha, "cwd": cwd, "tail": tail
            });
            if let Some(log_path) = argv.get(10) {
                params["logPath"] = json!(log_path);
            }
            Ok(("task.gate", params))
        }

        Some("challenge") => {
            let usage = "cli: task challenge <workspaceId> <slug> --claim t --evidence t --proposal t --default t [--deadline-min N]";
            let actor_id = argv.get(2).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let workspace_id = argv.get(3).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let slug = argv.get(4).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let rest = argv.get(5..).unwrap_or(&[]).to_vec();
            let (claim, rest) = take_flag(&rest, "--claim");
            let (evidence, rest) = take_flag(&rest, "--evidence");
            let (proposal, rest) = take_flag(&rest, "--proposal");
            let (default_action, rest) = take_flag(&rest, "--default");
            let (deadline_min, rest) = take_flag(&rest, "--deadline-min");
            if !rest.is_empty() {
                return Err(AppError::Invalid(usage.into()));
            }
            let (claim, evidence, proposal, default_action) = (
                claim.ok_or_else(|| AppError::Invalid(usage.into()))?,
                evidence.ok_or_else(|| AppError::Invalid(usage.into()))?,
                proposal.ok_or_else(|| AppError::Invalid(usage.into()))?,
                default_action.ok_or_else(|| AppError::Invalid(usage.into()))?,
            );
            let mut params = json!({
                "workspaceId": workspace_id, "slug": slug, "actorId": actor_id,
                "claim": claim, "evidence": evidence, "proposal": proposal,
                "default": default_action,
            });
            if let Some(deadline_min) = deadline_min {
                let parsed: i64 = deadline_min.parse().map_err(|_| {
                    AppError::Invalid(format!(
                        "cli: task challenge: bad --deadline-min '{deadline_min}'"
                    ))
                })?;
                params["deadlineMin"] = json!(parsed);
            }
            Ok(("task.challenge", params))
        }

        Some("rule") => {
            let usage = "cli: task rule <workspaceId> <slug> <challengeEventId> <text...>";
            let actor_id = argv.get(2).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let workspace_id = argv.get(3).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let slug = argv.get(4).ok_or_else(|| AppError::Invalid(usage.into()))?;
            let challenge_event_id = argv.get(5).ok_or_else(|| AppError::Invalid(usage.into()))?;
            if argv.len() < 7 {
                return Err(AppError::Invalid(usage.into()));
            }
            let text = argv[6..].join(" ");
            Ok((
                "task.rule",
                json!({
                    "workspaceId": workspace_id, "slug": slug, "actorId": actor_id,
                    "challengeEventId": challenge_event_id, "text": text
                }),
            ))
        }

        Some("close") => {
            let (actor_id, workspace_id, slug) = task_actor_ws_slug(argv, "task close")?;
            Ok((
                "task.close",
                json!({ "workspaceId": workspace_id, "slug": slug, "actorId": actor_id }),
            ))
        }

        Some("watch") => {
            let (actor_id, workspace_id, slug) = task_actor_ws_slug(argv, "task watch")?;
            Ok((
                "task.watch",
                json!({ "workspaceId": workspace_id, "slug": slug, "actorId": actor_id }),
            ))
        }

        Some("unwatch") => {
            let (actor_id, workspace_id, slug) = task_actor_ws_slug(argv, "task unwatch")?;
            Ok((
                "task.unwatch",
                json!({ "workspaceId": workspace_id, "slug": slug, "actorId": actor_id }),
            ))
        }

        _ => Err(AppError::Invalid(
            "cli: task <list|get|create|claim|plan-check|state|note|gate|challenge|rule|close|watch|unwatch> — unknown task subcommand".into(),
        )),
    }
}

/// Shared parse for the `task <verb> <actorId> <workspaceId> <slug>` shape
/// (`claim`/`close`/`watch`/`unwatch` — no further arguments).
fn task_actor_ws_slug<'a>(
    argv: &'a [String],
    usage_verb: &str,
) -> Result<(&'a str, &'a str, &'a str), AppError> {
    let usage = format!("cli: {usage_verb} <workspaceId> <slug>");
    let actor_id = argv
        .get(2)
        .ok_or_else(|| AppError::Invalid(usage.clone()))?;
    let workspace_id = argv
        .get(3)
        .ok_or_else(|| AppError::Invalid(usage.clone()))?;
    let slug = argv
        .get(4)
        .ok_or_else(|| AppError::Invalid(usage.clone()))?;
    if argv.len() != 5 {
        return Err(AppError::Invalid(usage));
    }
    Ok((actor_id, workspace_id, slug))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn ok_method(words: &[&str]) -> &'static str {
        map_argv(&argv(words)).expect("expected Ok").0
    }

    fn ok_params(words: &[&str]) -> Value {
        map_argv(&argv(words)).expect("expected Ok").1
    }

    fn is_invalid(words: &[&str]) -> bool {
        matches!(map_argv(&argv(words)), Err(AppError::Invalid(_)))
    }

    // ── ws ────────────────────────────────────────────────────────────────

    #[test]
    fn ws_list_maps_correctly() {
        assert_eq!(ok_method(&["ws", "list"]), "workspace.list");
        assert_eq!(ok_params(&["ws", "list"]), Value::Null);
    }

    #[test]
    fn ws_use_maps_correctly() {
        assert_eq!(ok_method(&["ws", "use", "abc"]), "workspace.use");
        assert_eq!(
            ok_params(&["ws", "use", "abc"]),
            json!({ "workspaceId": "abc" })
        );
    }

    #[test]
    fn ws_list_rejects_extra_args() {
        assert!(is_invalid(&["ws", "list", "extra"]));
    }

    #[test]
    fn ws_use_missing_id_is_invalid() {
        assert!(is_invalid(&["ws", "use"]));
    }

    #[test]
    fn ws_use_extra_args_is_invalid() {
        assert!(is_invalid(&["ws", "use", "id", "extra"]));
    }

    #[test]
    fn ws_unknown_sub_is_invalid() {
        assert!(is_invalid(&["ws", "delete"]));
    }

    // ── agent ─────────────────────────────────────────────────────────────

    #[test]
    fn agent_list_maps_correctly() {
        assert_eq!(ok_method(&["agent", "list", "ws1"]), "instance.list");
        assert_eq!(
            ok_params(&["agent", "list", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
    }

    #[test]
    fn agent_list_missing_workspace_is_invalid() {
        assert!(is_invalid(&["agent", "list"]));
    }

    #[test]
    fn agent_list_extra_args_is_invalid() {
        assert!(is_invalid(&["agent", "list", "ws1", "extra"]));
    }

    #[test]
    fn agent_unknown_sub_is_invalid() {
        assert!(is_invalid(&["agent", "spawn"]));
    }

    // ── send ──────────────────────────────────────────────────────────────

    #[test]
    fn send_single_word_maps_correctly() {
        assert_eq!(ok_method(&["send", "sess1", "hello"]), "message.send");
        assert_eq!(
            ok_params(&["send", "sess1", "hello"]),
            json!({ "sessionId": "sess1", "text": "hello" })
        );
    }

    #[test]
    fn send_multi_word_text_joined_with_spaces() {
        let params = ok_params(&["send", "sess1", "hello", "world", "foo"]);
        assert_eq!(params["text"], json!("hello world foo"));
        assert_eq!(params["sessionId"], json!("sess1"));
    }

    #[test]
    fn send_missing_text_is_invalid() {
        assert!(is_invalid(&["send", "sess1"]));
    }

    #[test]
    fn send_missing_session_is_invalid() {
        assert!(is_invalid(&["send"]));
    }

    // ── tell ──────────────────────────────────────────────────────────────

    #[test]
    fn tell_maps_to_inject() {
        assert_eq!(ok_method(&["tell", "from1", "to1", "hi"]), "message.inject");
        assert_eq!(
            ok_params(&["tell", "from1", "to1", "hi"]),
            json!({ "fromInstanceId": "from1", "toInstanceId": "to1", "text": "hi" })
        );
    }

    #[test]
    fn tell_joins_multiword_text() {
        let params = ok_params(&["tell", "from1", "to1", "hello", "there", "peer"]);
        assert_eq!(params["text"], json!("hello there peer"));
    }

    #[test]
    fn tell_missing_text_is_invalid() {
        assert!(is_invalid(&["tell", "from1", "to1"]));
    }

    #[test]
    fn tell_missing_target_is_invalid() {
        assert!(is_invalid(&["tell", "from1"]));
    }

    // ── msg (read inter-agent history) ─────────────────────────────────────
    // `msg list` arrives ALREADY expanded by the client with the caller's own
    // instance id at argv[2]; `msg all` passes through with an explicit ws id.

    #[test]
    fn msg_list_maps_to_message_list_with_names() {
        assert_eq!(ok_method(&["msg", "list", "self1"]), "message.list");
        assert_eq!(
            ok_params(&["msg", "list", "self1"]),
            json!({ "instanceId": "self1", "withNames": true })
        );
    }

    #[test]
    fn msg_list_with_limit() {
        let p = ok_params(&["msg", "list", "self1", "--limit", "5"]);
        assert_eq!(p["instanceId"], json!("self1"));
        assert_eq!(p["withNames"], json!(true));
        assert_eq!(p["limit"], json!(5));
    }

    #[test]
    fn msg_all_maps_to_list_for_workspace_with_names() {
        assert_eq!(
            ok_method(&["msg", "all", "ws1"]),
            "message.listForWorkspace"
        );
        assert_eq!(
            ok_params(&["msg", "all", "ws1"]),
            json!({ "workspaceId": "ws1", "withNames": true })
        );
    }

    #[test]
    fn msg_all_with_limit() {
        let p = ok_params(&["msg", "all", "ws1", "--limit", "10"]);
        assert_eq!(p["workspaceId"], json!("ws1"));
        assert_eq!(p["withNames"], json!(true));
        assert_eq!(p["limit"], json!(10));
    }

    #[test]
    fn msg_missing_or_bad_args_are_invalid() {
        assert!(is_invalid(&["msg"]));
        assert!(is_invalid(&["msg", "list"])); // no instance id (client injects self)
        assert!(is_invalid(&["msg", "all"])); // no workspace id
        assert!(is_invalid(&["msg", "bogus", "x"])); // unknown sub
        assert!(is_invalid(&["msg", "list", "self1", "--limit"])); // dangling --limit
        assert!(is_invalid(&["msg", "list", "self1", "--limit", "x"])); // non-integer
        assert!(is_invalid(&["msg", "list", "self1", "extra"])); // trailing junk
        assert!(is_invalid(&["msg", "all", "ws1", "extra"])); // trailing junk
    }

    // ── orient (task conclave-orient) ───────────────────────────────────────

    #[test]
    fn orient_maps_correctly() {
        assert_eq!(ok_method(&["orient", "self1", "ws1"]), "orient.list");
        assert_eq!(
            ok_params(&["orient", "self1", "ws1"]),
            json!({ "workspaceId": "ws1", "actorId": "self1" })
        );
    }

    #[test]
    fn orient_wrong_arity_is_invalid() {
        assert!(is_invalid(&["orient"]));
        assert!(is_invalid(&["orient", "self1"])); // no workspace (client injects self)
        assert!(is_invalid(&["orient", "self1", "ws1", "extra"])); // trailing junk
    }

    // ── bb ────────────────────────────────────────────────────────────────

    #[test]
    fn bb_list_maps_correctly() {
        assert_eq!(ok_method(&["bb", "list", "ws1"]), "blackboard.list");
        assert_eq!(
            ok_params(&["bb", "list", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
    }

    #[test]
    fn bb_list_missing_workspace_is_invalid() {
        assert!(is_invalid(&["bb", "list"]));
    }

    #[test]
    fn bb_list_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "list", "ws1", "extra"]));
    }

    #[test]
    fn bb_get_maps_correctly() {
        assert_eq!(ok_method(&["bb", "get", "ws1", "mykey"]), "blackboard.get");
        assert_eq!(
            ok_params(&["bb", "get", "ws1", "mykey"]),
            json!({ "workspaceId": "ws1", "key": "mykey" })
        );
    }

    #[test]
    fn bb_get_missing_key_is_invalid() {
        assert!(is_invalid(&["bb", "get", "ws1"]));
    }

    #[test]
    fn bb_get_missing_workspace_is_invalid() {
        assert!(is_invalid(&["bb", "get"]));
    }

    #[test]
    fn bb_get_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "get", "ws1", "key", "extra"]));
    }

    #[test]
    fn bb_delete_maps_correctly_with_rm_alias() {
        assert_eq!(
            ok_method(&["bb", "delete", "ws1", "mykey"]),
            "blackboard.delete"
        );
        assert_eq!(
            ok_params(&["bb", "delete", "ws1", "mykey"]),
            json!({ "workspaceId": "ws1", "key": "mykey" })
        );
        assert_eq!(
            ok_method(&["bb", "rm", "ws1", "mykey"]),
            "blackboard.delete"
        );
    }

    #[test]
    fn bb_delete_missing_or_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "delete", "ws1"]));
        assert!(is_invalid(&["bb", "delete"]));
        assert!(is_invalid(&["bb", "delete", "ws1", "key", "extra"]));
    }

    #[test]
    fn bb_set_maps_correctly() {
        assert_eq!(ok_method(&["bb", "set", "ws1", "k", "v"]), "blackboard.set");
        let params = ok_params(&["bb", "set", "ws1", "k", "v"]);
        assert_eq!(params["workspaceId"], json!("ws1"));
        assert_eq!(params["key"], json!("k"));
        assert_eq!(params["value"], json!("v"));
    }

    #[test]
    fn bb_set_missing_value_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1", "k"]));
    }

    #[test]
    fn bb_set_missing_key_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1"]));
    }

    #[test]
    fn bb_set_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1", "k", "v", "extra"]));
    }

    #[test]
    fn bb_unknown_sub_is_invalid() {
        // `delete` used to be the unknown-subcommand fixture here — it's a real
        // subcommand now (see bb_delete_maps_correctly_with_rm_alias).
        assert!(is_invalid(&["bb", "purge", "ws1", "k"]));
    }

    /// `bb set` JSON value coercion: numeric string → JSON number.
    #[test]
    fn bb_set_numeric_string_becomes_number() {
        let params = ok_params(&["bb", "set", "ws1", "k", "42"]);
        assert_eq!(params["value"], json!(42));
    }

    /// `bb set` JSON value coercion: `"true"` → JSON boolean.
    #[test]
    fn bb_set_bool_string_becomes_bool() {
        let params = ok_params(&["bb", "set", "ws1", "k", "true"]);
        assert_eq!(params["value"], json!(true));
    }

    /// `bb set` JSON value coercion: bare word → JSON string.
    #[test]
    fn bb_set_bare_word_becomes_string() {
        let params = ok_params(&["bb", "set", "ws1", "k", "hello"]);
        assert_eq!(params["value"], json!("hello"));
    }

    /// `bb set` JSON value coercion: valid JSON object stays an object.
    #[test]
    fn bb_set_json_object_stays_object() {
        let params = ok_params(&["bb", "set", "ws1", "k", r#"{"a":1}"#]);
        assert_eq!(params["value"], json!({ "a": 1 }));
    }

    // ── snapshot ──────────────────────────────────────────────────────────

    #[test]
    fn snapshot_list_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "list", "sess1"]), "snapshot.list");
        assert_eq!(
            ok_params(&["snapshot", "list", "sess1"]),
            json!({ "sessionId": "sess1" })
        );
    }

    #[test]
    fn snapshot_list_missing_session_is_invalid() {
        assert!(is_invalid(&["snapshot", "list"]));
    }

    #[test]
    fn snapshot_list_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "list", "s1", "extra"]));
    }

    #[test]
    fn snapshot_read_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "read", "snap1"]), "snapshot.read");
        assert_eq!(
            ok_params(&["snapshot", "read", "snap1"]),
            json!({ "snapshotId": "snap1" })
        );
    }

    #[test]
    fn snapshot_read_missing_id_is_invalid() {
        assert!(is_invalid(&["snapshot", "read"]));
    }

    #[test]
    fn snapshot_read_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "read", "s1", "extra"]));
    }

    #[test]
    fn snapshot_create_without_label_maps_correctly() {
        assert_eq!(
            ok_method(&["snapshot", "create", "sess1", "auto"]),
            "snapshot.create"
        );
        let params = ok_params(&["snapshot", "create", "sess1", "auto"]);
        assert_eq!(params["sessionId"], json!("sess1"));
        assert_eq!(params["type"], json!("auto"));
        assert!(params.get("label").is_none(), "label absent when not given");
    }

    #[test]
    fn snapshot_create_with_label_maps_correctly() {
        let params = ok_params(&["snapshot", "create", "sess1", "manual", "my-label"]);
        assert_eq!(params["sessionId"], json!("sess1"));
        assert_eq!(params["type"], json!("manual"));
        assert_eq!(params["label"], json!("my-label"));
    }

    #[test]
    fn snapshot_create_missing_type_is_invalid() {
        assert!(is_invalid(&["snapshot", "create", "sess1"]));
    }

    #[test]
    fn snapshot_create_extra_args_is_invalid() {
        assert!(is_invalid(&[
            "snapshot", "create", "sess1", "auto", "label", "extra"
        ]));
    }

    #[test]
    fn snapshot_create_missing_session_is_invalid() {
        assert!(is_invalid(&["snapshot", "create"]));
    }

    #[test]
    fn snapshot_unknown_sub_is_invalid() {
        assert!(is_invalid(&["snapshot", "delete", "s1"]));
    }

    #[test]
    fn snapshot_save_maps_correctly() {
        assert_eq!(
            ok_method(&["snapshot", "save", "inst1", "my", "handoff"]),
            "snapshot.save"
        );
        let params = ok_params(&["snapshot", "save", "inst1", "my", "handoff"]);
        assert_eq!(params["instanceId"], json!("inst1"));
        assert_eq!(params["text"], json!("my handoff"));
    }

    #[test]
    fn snapshot_save_missing_text_is_invalid() {
        assert!(is_invalid(&["snapshot", "save", "inst1"]));
    }

    #[test]
    fn snapshot_last_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "last", "inst1"]), "snapshot.last");
        assert_eq!(
            ok_params(&["snapshot", "last", "inst1"]),
            json!({ "instanceId": "inst1" })
        );
    }

    #[test]
    fn snapshot_last_missing_instance_is_invalid() {
        assert!(is_invalid(&["snapshot", "last"]));
    }

    #[test]
    fn snapshot_last_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "last", "inst1", "extra"]));
    }

    // ── memory ────────────────────────────────────────────────────────────

    #[test]
    fn memory_remember_maps_correctly_without_author() {
        // "-" sentinel: `conclave-cli` injects this outside a spawned agent.
        assert_eq!(
            ok_method(&["memory", "remember", "-", "ws1", "hello"]),
            "memory.remember"
        );
        assert_eq!(
            ok_params(&["memory", "remember", "-", "ws1", "hello"]),
            json!({ "workspaceId": "ws1", "text": "hello" })
        );
    }

    #[test]
    fn memory_remember_stamps_agent_author_when_present() {
        let params = ok_params(&["memory", "remember", "inst1", "ws1", "hello"]);
        assert_eq!(
            params,
            json!({
                "workspaceId": "ws1",
                "text": "hello",
                "sourceKind": "agent",
                "sourceId": "inst1",
            })
        );
    }

    #[test]
    fn memory_remember_joins_multiword_text() {
        let params = ok_params(&["memory", "remember", "-", "ws1", "hello", "there", "world"]);
        assert_eq!(params["text"], json!("hello there world"));
    }

    #[test]
    fn memory_remember_missing_text_is_invalid() {
        assert!(is_invalid(&["memory", "remember", "-", "ws1"]));
    }

    #[test]
    fn memory_remember_missing_workspace_is_invalid() {
        assert!(is_invalid(&["memory", "remember", "-"]));
    }

    #[test]
    fn memory_remember_missing_author_is_invalid() {
        assert!(is_invalid(&["memory", "remember"]));
    }

    #[test]
    fn memory_search_maps_correctly_without_limit() {
        assert_eq!(
            ok_method(&["memory", "search", "ws1", "hello", "world"]),
            "memory.search"
        );
        let params = ok_params(&["memory", "search", "ws1", "hello", "world"]);
        assert_eq!(params["workspaceId"], json!("ws1"));
        assert_eq!(params["query"], json!("hello world"));
        assert!(params.get("limit").is_none(), "limit absent when not given");
    }

    #[test]
    fn memory_search_strips_trailing_limit_flag() {
        let params = ok_params(&["memory", "search", "ws1", "hello", "world", "--limit", "3"]);
        assert_eq!(params["query"], json!("hello world"));
        assert_eq!(params["limit"], json!(3));
    }

    #[test]
    fn memory_search_single_word_query_with_limit() {
        let params = ok_params(&["memory", "search", "ws1", "hello", "--limit", "5"]);
        assert_eq!(params["query"], json!("hello"));
        assert_eq!(params["limit"], json!(5));
    }

    #[test]
    fn memory_search_non_integer_limit_is_invalid() {
        assert!(is_invalid(&[
            "memory", "search", "ws1", "hello", "--limit", "abc"
        ]));
    }

    #[test]
    fn memory_search_limit_flag_without_query_is_invalid() {
        // `--limit 3` alone, with nothing left over for the query.
        assert!(is_invalid(&["memory", "search", "ws1", "--limit", "3"]));
    }

    #[test]
    fn memory_search_missing_query_is_invalid() {
        assert!(is_invalid(&["memory", "search", "ws1"]));
    }

    #[test]
    fn memory_search_missing_workspace_is_invalid() {
        assert!(is_invalid(&["memory", "search"]));
    }

    #[test]
    fn memory_search_literal_dash_dash_limit_word_in_query_is_not_stripped_without_a_value() {
        // "--limit" as the very LAST word (no value following it) must stay
        // part of the query rather than being (invalidly) treated as a flag.
        let params = ok_params(&["memory", "search", "ws1", "--limit"]);
        assert_eq!(params["query"], json!("--limit"));
        assert!(params.get("limit").is_none());
    }

    #[test]
    fn memory_delete_maps_correctly() {
        assert_eq!(
            ok_method(&["memory", "delete", "ws1", "chunk1"]),
            "memory.delete"
        );
        assert_eq!(
            ok_params(&["memory", "delete", "ws1", "chunk1"]),
            json!({ "workspaceId": "ws1", "id": "chunk1" })
        );
    }

    #[test]
    fn memory_delete_missing_or_extra_args_is_invalid() {
        assert!(is_invalid(&["memory", "delete", "ws1"]));
        assert!(is_invalid(&["memory", "delete"]));
        assert!(is_invalid(&["memory", "delete", "ws1", "chunk1", "extra"]));
    }

    #[test]
    fn memory_status_maps_correctly() {
        assert_eq!(ok_method(&["memory", "status", "ws1"]), "memory.status");
        assert_eq!(
            ok_params(&["memory", "status", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
    }

    #[test]
    fn memory_status_missing_or_extra_args_is_invalid() {
        assert!(is_invalid(&["memory", "status"]));
        assert!(is_invalid(&["memory", "status", "ws1", "extra"]));
    }

    #[test]
    fn memory_unknown_sub_is_invalid() {
        assert!(is_invalid(&["memory", "clear", "ws1"]));
    }

    // ── memory review queue (plan memory-distill-queue) ─────────────────────
    // These arrive already expanded by `conclave-cli`'s `expand_self_args`: the
    // proposer/reviewer slot right after the verb is the caller's own agent id.

    #[test]
    fn memory_propose_maps_with_and_without_source_note() {
        assert_eq!(
            ok_method(&["memory", "propose", "agent1", "ws1", "a", "fact"]),
            "memory.propose"
        );
        assert_eq!(
            ok_params(&["memory", "propose", "agent1", "ws1", "a", "fact"]),
            json!({ "workspaceId": "ws1", "proposerId": "agent1", "text": "a fact" })
        );
        assert_eq!(
            ok_params(&[
                "memory",
                "propose",
                "agent1",
                "ws1",
                "a",
                "fact",
                "--source-note",
                "t.jsonl",
            ]),
            json!({
                "workspaceId": "ws1",
                "proposerId": "agent1",
                "text": "a fact",
                "sourceNote": "t.jsonl",
            })
        );
    }

    #[test]
    fn memory_propose_missing_text_is_invalid() {
        assert!(is_invalid(&["memory", "propose", "agent1", "ws1"]));
        assert!(is_invalid(&["memory", "propose", "agent1"]));
        // a `--source-note` with no remaining text is not a valid proposal
        assert!(is_invalid(&[
            "memory",
            "propose",
            "agent1",
            "ws1",
            "--source-note",
            "t.jsonl",
        ]));
    }

    #[test]
    fn memory_queue_maps_with_and_without_state() {
        assert_eq!(ok_method(&["memory", "queue", "ws1"]), "memory.queue");
        assert_eq!(
            ok_params(&["memory", "queue", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
        assert_eq!(
            ok_params(&["memory", "queue", "ws1", "--state", "approved"]),
            json!({ "workspaceId": "ws1", "state": "approved" })
        );
    }

    #[test]
    fn memory_queue_bad_flag_is_invalid() {
        assert!(is_invalid(&["memory", "queue"]));
        assert!(is_invalid(&["memory", "queue", "ws1", "--bogus", "x"]));
        assert!(is_invalid(&["memory", "queue", "ws1", "extra"]));
    }

    #[test]
    fn memory_approve_and_reject_map_with_optional_reason() {
        assert_eq!(
            ok_method(&["memory", "approve", "rev1", "ws1", "p1"]),
            "memory.approve"
        );
        assert_eq!(
            ok_params(&["memory", "approve", "rev1", "ws1", "p1"]),
            json!({ "workspaceId": "ws1", "reviewerId": "rev1", "proposalId": "p1" })
        );
        assert_eq!(
            ok_method(&["memory", "reject", "rev1", "ws1", "p1", "--reason", "in", "the", "docs"]),
            "memory.reject"
        );
        assert_eq!(
            ok_params(&["memory", "reject", "rev1", "ws1", "p1", "--reason", "in", "the", "docs"]),
            json!({
                "workspaceId": "ws1",
                "reviewerId": "rev1",
                "proposalId": "p1",
                "reason": "in the docs",
            })
        );
    }

    #[test]
    fn memory_approve_missing_args_or_bad_reason_is_invalid() {
        assert!(is_invalid(&["memory", "approve", "rev1", "ws1"]));
        assert!(is_invalid(&["memory", "approve", "rev1"]));
        // a lone `--reason` with no text, or a stray non-flag trailing token
        assert!(is_invalid(&[
            "memory", "approve", "rev1", "ws1", "p1", "--reason"
        ]));
        assert!(is_invalid(&[
            "memory", "reject", "rev1", "ws1", "p1", "stray"
        ]));
    }

    // ── task (ADR 0008) ─────────────────────────────────────────────────────

    #[test]
    fn task_list_defaults_to_slim_and_full_restores_plan_shape() {
        assert_eq!(ok_method(&["task", "list", "ws1"]), "task.list");
        assert_eq!(
            ok_params(&["task", "list", "ws1"]),
            json!({ "workspaceId": "ws1", "slim": true })
        );
        assert_eq!(
            ok_params(&["task", "list", "ws1", "--state", "claimed"]),
            json!({ "workspaceId": "ws1", "state": "claimed", "slim": true })
        );
        assert_eq!(
            ok_params(&["task", "list", "ws1", "--full"]),
            json!({ "workspaceId": "ws1", "slim": false, "includePlan": true })
        );
        assert_eq!(
            ok_params(&["task", "list", "ws1", "--state", "claimed", "--full"]),
            json!({ "workspaceId": "ws1", "state": "claimed", "slim": false, "includePlan": true })
        );
    }

    #[test]
    fn task_list_all_stays_slim_but_includes_closed() {
        assert_eq!(
            ok_params(&["task", "list", "ws1", "--all"]),
            json!({ "workspaceId": "ws1", "slim": true, "all": true })
        );
        assert_eq!(
            ok_params(&["task", "list", "ws1", "--state", "merged", "--all"]),
            json!({ "workspaceId": "ws1", "state": "merged", "slim": true, "all": true })
        );
    }

    #[test]
    fn task_list_extra_args_is_invalid() {
        assert!(is_invalid(&["task", "list", "ws1", "extra"]));
        assert!(is_invalid(&["task", "list", "ws1", "--full", "extra"]));
        // Contradictory: --full already includes every state.
        assert!(is_invalid(&["task", "list", "ws1", "--full", "--all"]));
    }

    #[test]
    fn task_get_maps_correctly() {
        assert_eq!(ok_method(&["task", "get", "ws1", "t1"]), "task.get");
        assert_eq!(
            ok_params(&["task", "get", "ws1", "t1"]),
            json!({ "workspaceId": "ws1", "slug": "t1" })
        );
    }

    #[test]
    fn task_get_missing_slug_is_invalid() {
        assert!(is_invalid(&["task", "get", "ws1"]));
    }

    #[test]
    fn task_brief_maps_correctly() {
        assert_eq!(ok_method(&["task", "brief", "ws1", "t1"]), "task.brief");
        assert_eq!(
            ok_params(&["task", "brief", "ws1", "t1"]),
            json!({ "workspaceId": "ws1", "slug": "t1" })
        );
        assert_eq!(
            ok_params(&["task", "brief", "ws1", "t1", "--limit", "3"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "limit": 3 })
        );
    }

    #[test]
    fn task_brief_missing_args_or_bad_limit_is_invalid() {
        assert!(is_invalid(&["task", "brief", "ws1"]));
        assert!(is_invalid(&["task", "brief", "ws1", "t1", "--limit"]));
        assert!(is_invalid(&[
            "task", "brief", "ws1", "t1", "--limit", "abc"
        ]));
    }

    #[test]
    fn task_create_maps_correctly_bare() {
        assert_eq!(
            ok_method(&["task", "create", "ws1", "t1", "My", "Title"]),
            "task.create"
        );
        assert_eq!(
            ok_params(&["task", "create", "ws1", "t1", "My", "Title"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "title": "My Title" })
        );
    }

    #[test]
    fn task_create_flags_are_order_independent_and_dont_pollute_title() {
        let params = ok_params(&[
            "task",
            "create",
            "ws1",
            "t1",
            "--owner",
            "agent-1",
            "Title",
            "Words",
            "--boundary",
            "a.rs,b.rs",
            "--canon",
            "canon-x",
            "--plan",
            "do it",
        ]);
        assert_eq!(params["title"], json!("Title Words"));
        assert_eq!(params["ownerAgentId"], json!("agent-1"));
        assert_eq!(params["fileBoundary"], json!(["a.rs", "b.rs"]));
        assert_eq!(params["designCanon"], json!("canon-x"));
        assert_eq!(params["plan"], json!("do it"));
    }

    #[test]
    fn task_create_watchers_maps_to_watcher_agent_ids_and_drops_empties() {
        let params = ok_params(&[
            "task",
            "create",
            "ws1",
            "t1",
            "Title",
            "--watchers",
            "a,,b,",
            "--owner",
            "agent-1",
        ]);
        // Comma-split, empties dropped — mirrors `--boundary`.
        assert_eq!(params["watcherAgentIds"], json!(["a", "b"]));
        assert_eq!(params["title"], json!("Title"));
        assert_eq!(params["ownerAgentId"], json!("agent-1"));
    }

    #[test]
    fn task_create_without_watchers_omits_the_field() {
        // Byte-for-byte: the flag-less create must not carry watcherAgentIds.
        let params = ok_params(&["task", "create", "ws1", "t1", "My", "Title"]);
        assert_eq!(
            params,
            json!({ "workspaceId": "ws1", "slug": "t1", "title": "My Title" })
        );
        assert!(params.get("watcherAgentIds").is_none());
    }

    #[test]
    fn task_create_missing_title_is_invalid() {
        assert!(is_invalid(&["task", "create", "ws1", "t1"]));
        assert!(is_invalid(&[
            "task", "create", "ws1", "t1", "--owner", "a1"
        ]));
    }

    #[test]
    fn task_claim_maps_correctly() {
        assert_eq!(
            ok_method(&["task", "claim", "actor1", "ws1", "t1"]),
            "task.claim"
        );
        assert_eq!(
            ok_params(&["task", "claim", "actor1", "ws1", "t1"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "actorId": "actor1" })
        );
    }

    #[test]
    fn task_claim_missing_args_is_invalid() {
        assert!(is_invalid(&["task", "claim", "actor1", "ws1"]));
        assert!(is_invalid(&[
            "task", "claim", "actor1", "ws1", "t1", "extra"
        ]));
    }

    #[test]
    fn task_plan_check_maps_correctly() {
        let words = [
            "task",
            "plan-check",
            "actor1",
            "ws1",
            "t1",
            "docs/plans/x.md",
            "deadbeef",
            "1",
            "# Plan\ncontent",
            r#"{"src/a.rs":"pub fn a() {}"}"#,
        ];
        assert_eq!(ok_method(&words), "task.planCheck");
        assert_eq!(
            ok_params(&words),
            json!({
                "workspaceId": "ws1", "slug": "t1", "actorId": "actor1",
                "planPath": "docs/plans/x.md", "planFingerprint": "deadbeef",
                "baseShaAncestor": true, "planContent": "# Plan\ncontent",
                "files": { "src/a.rs": "pub fn a() {}" },
            })
        );
        // Ancestor flag "0" maps to false.
        let mut not_ancestor = words;
        not_ancestor[7] = "0";
        assert_eq!(ok_params(&not_ancestor)["baseShaAncestor"], json!(false));
    }

    #[test]
    fn task_plan_check_wrong_arity_or_bad_words_is_invalid() {
        // Too short (the raw 2-arg CLI form must never reach the wire).
        assert!(is_invalid(&["task", "plan-check", "ws1", "t1"]));
        // Too long.
        assert!(is_invalid(&[
            "task",
            "plan-check",
            "actor1",
            "ws1",
            "t1",
            "docs/x.md",
            "fp",
            "1",
            "plan",
            "{}",
            "extra"
        ]));
        // Bad ancestor flag.
        assert!(is_invalid(&[
            "task",
            "plan-check",
            "actor1",
            "ws1",
            "t1",
            "docs/x.md",
            "fp",
            "yes",
            "plan",
            "{}"
        ]));
        // Files word must parse as a JSON object.
        assert!(is_invalid(&[
            "task",
            "plan-check",
            "actor1",
            "ws1",
            "t1",
            "docs/x.md",
            "fp",
            "1",
            "plan",
            "not json"
        ]));
        assert!(is_invalid(&[
            "task",
            "plan-check",
            "actor1",
            "ws1",
            "t1",
            "docs/x.md",
            "fp",
            "1",
            "plan",
            "[]"
        ]));
    }

    #[test]
    fn task_state_maps_correctly() {
        assert_eq!(
            ok_method(&["task", "state", "actor1", "ws1", "t1", "in_progress"]),
            "task.setState"
        );
        assert_eq!(
            ok_params(&["task", "state", "actor1", "ws1", "t1", "in_progress"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "state": "in_progress", "actorId": "actor1" })
        );
    }

    #[test]
    fn task_note_joins_multiword_text() {
        assert_eq!(
            ok_method(&["task", "note", "actor1", "ws1", "t1", "hello", "world"]),
            "task.note"
        );
        let params = ok_params(&["task", "note", "actor1", "ws1", "t1", "hello", "world"]);
        assert_eq!(params["text"], json!("hello world"));
        assert_eq!(params["actorId"], json!("actor1"));
    }

    #[test]
    fn task_note_missing_text_is_invalid() {
        assert!(is_invalid(&["task", "note", "actor1", "ws1", "t1"]));
    }

    #[test]
    fn task_gate_maps_correctly() {
        assert_eq!(
            ok_method(&[
                "task",
                "gate",
                "actor1",
                "ws1",
                "t1",
                "cargo test",
                "0",
                "sha123",
                "/repo",
                "ok"
            ]),
            "task.gate"
        );
        let params = ok_params(&[
            "task",
            "gate",
            "actor1",
            "ws1",
            "t1",
            "cargo test",
            "101",
            "sha123",
            "/repo",
            "FAILED",
        ]);
        assert_eq!(params["cmd"], json!("cargo test"));
        assert_eq!(params["exit"], json!(101));
        assert_eq!(params["sha"], json!("sha123"));
        assert_eq!(params["cwd"], json!("/repo"));
        assert_eq!(params["tail"], json!("FAILED"));
        assert_eq!(params["actorId"], json!("actor1"));
        // The 10-word (pre-logPath) form must not grow a logPath key.
        assert!(params.get("logPath").is_none());
    }

    #[test]
    fn task_gate_eleventh_word_maps_to_log_path() {
        let params = ok_params(&[
            "task",
            "gate",
            "actor1",
            "ws1",
            "t1",
            "cargo test",
            "0",
            "sha123",
            "/repo",
            "ok",
            "/logs/gate-t1.log",
        ]);
        assert_eq!(params["logPath"], json!("/logs/gate-t1.log"));
        assert_eq!(params["tail"], json!("ok"));
        // Twelve words is stray input, not a longer gate form.
        assert!(is_invalid(&[
            "task", "gate", "actor1", "ws1", "t1", "cmd", "0", "sha", "/repo", "tail", "/log",
            "extra"
        ]));
    }

    #[test]
    fn task_gate_bad_exit_code_is_invalid() {
        assert!(is_invalid(&[
            "task",
            "gate",
            "actor1",
            "ws1",
            "t1",
            "cmd",
            "notanumber",
            "sha",
            "/repo",
            "tail"
        ]));
    }

    #[test]
    fn task_gate_wrong_arity_is_invalid() {
        assert!(is_invalid(&["task", "gate", "actor1", "ws1", "t1"]));
    }

    #[test]
    fn task_challenge_maps_correctly() {
        let params = ok_params(&[
            "task",
            "challenge",
            "actor1",
            "ws1",
            "t1",
            "--claim",
            "X broken",
            "--evidence",
            "log",
            "--proposal",
            "fix Y",
            "--default",
            "escalate",
            "--deadline-min",
            "30",
        ]);
        assert_eq!(params["claim"], json!("X broken"));
        assert_eq!(params["evidence"], json!("log"));
        assert_eq!(params["proposal"], json!("fix Y"));
        assert_eq!(params["default"], json!("escalate"));
        assert_eq!(params["deadlineMin"], json!(30));
        assert_eq!(params["actorId"], json!("actor1"));
    }

    #[test]
    fn task_challenge_without_deadline_omits_it() {
        let params = ok_params(&[
            "task",
            "challenge",
            "actor1",
            "ws1",
            "t1",
            "--claim",
            "c",
            "--evidence",
            "e",
            "--proposal",
            "p",
            "--default",
            "d",
        ]);
        assert!(params.get("deadlineMin").is_none());
    }

    #[test]
    fn task_challenge_missing_required_flag_is_invalid() {
        assert!(is_invalid(&[
            "task",
            "challenge",
            "actor1",
            "ws1",
            "t1",
            "--claim",
            "c",
            "--evidence",
            "e",
            "--proposal",
            "p"
        ]));
    }

    #[test]
    fn task_rule_maps_correctly() {
        assert_eq!(
            ok_method(&["task", "rule", "actor1", "ws1", "t1", "ev1", "go", "with", "it"]),
            "task.rule"
        );
        let params = ok_params(&[
            "task", "rule", "actor1", "ws1", "t1", "ev1", "go", "with", "it",
        ]);
        assert_eq!(params["challengeEventId"], json!("ev1"));
        assert_eq!(params["text"], json!("go with it"));
        assert_eq!(params["actorId"], json!("actor1"));
    }

    #[test]
    fn task_rule_missing_text_is_invalid() {
        assert!(is_invalid(&["task", "rule", "actor1", "ws1", "t1", "ev1"]));
    }

    #[test]
    fn task_close_maps_correctly() {
        assert_eq!(
            ok_method(&["task", "close", "actor1", "ws1", "t1"]),
            "task.close"
        );
        assert_eq!(
            ok_params(&["task", "close", "actor1", "ws1", "t1"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "actorId": "actor1" })
        );
    }

    #[test]
    fn task_watch_and_unwatch_map_correctly() {
        assert_eq!(
            ok_method(&["task", "watch", "actor1", "ws1", "t1"]),
            "task.watch"
        );
        assert_eq!(
            ok_method(&["task", "unwatch", "actor1", "ws1", "t1"]),
            "task.unwatch"
        );
        assert_eq!(
            ok_params(&["task", "watch", "actor1", "ws1", "t1"]),
            json!({ "workspaceId": "ws1", "slug": "t1", "actorId": "actor1" })
        );
    }

    #[test]
    fn task_unknown_sub_is_invalid() {
        assert!(is_invalid(&["task", "nope", "ws1"]));
    }

    #[tokio::test]
    async fn exec_task_create_then_get_round_trips_through_router() {
        let state = AppState::for_tests().await;
        let ws = crate::engine::repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id;

        let created = exec(
            &state,
            json!({ "argv": ["task", "create", ws, "t1", "My", "Title"] }),
        )
        .await
        .expect("exec task create should succeed");
        assert_eq!(created["slug"], json!("t1"));

        let got = exec(&state, json!({ "argv": ["task", "get", ws, "t1"] }))
            .await
            .expect("exec task get should succeed");
        assert_eq!(got["task"]["slug"], json!("t1"));
        assert_eq!(got["events"], json!([]));
    }

    // ── run ───────────────────────────────────────────────────────────────

    #[test]
    fn run_single_word_prompt_maps_correctly() {
        assert_eq!(ok_method(&["run", "orch1", "go"]), "fusion.run");
        assert_eq!(
            ok_params(&["run", "orch1", "go"]),
            json!({ "orchestratorId": "orch1", "prompt": "go" })
        );
    }

    #[test]
    fn run_multi_word_prompt_joined_with_spaces() {
        let params = ok_params(&["run", "orch1", "do", "the", "thing"]);
        assert_eq!(params["orchestratorId"], json!("orch1"));
        assert_eq!(params["prompt"], json!("do the thing"));
    }

    #[test]
    fn run_missing_prompt_is_invalid() {
        assert!(is_invalid(&["run", "orch1"]));
    }

    #[test]
    fn run_missing_orchestrator_is_invalid() {
        assert!(is_invalid(&["run"]));
    }

    // ── restart (ADR 0006: self-triggered restart) ─────────────────────────

    #[test]
    fn restart_maps_correctly() {
        assert_eq!(ok_method(&["restart", "inst1"]), "instance.restart");
        assert_eq!(
            ok_params(&["restart", "inst1"]),
            json!({ "workspaceAgentId": "inst1", "self": true })
        );
    }

    #[test]
    fn restart_missing_instance_id_is_invalid() {
        assert!(is_invalid(&["restart"]));
    }

    #[test]
    fn restart_extra_args_is_invalid() {
        assert!(is_invalid(&["restart", "inst1", "extra"]));
    }

    // ── empty argv ────────────────────────────────────────────────────────

    #[test]
    fn empty_argv_is_invalid() {
        assert!(is_invalid(&[]));
    }

    // ── unknown top-level subcommand ──────────────────────────────────────

    #[test]
    fn unknown_subcommand_is_invalid() {
        assert!(is_invalid(&["nope"]));
    }

    // ── artifact (plan design-artifact-store) ─────────────────────────────
    // `add` argv arrives already expanded by conclave-cli: the creator-agent
    // slot sits at argv[2] ("-" outside a spawned agent), --file already
    // resolved to --content client-side.

    #[test]
    fn artifact_add_maps_with_agent_and_flags() {
        let words = &[
            "artifact",
            "add",
            "self1",
            "ws1",
            "--title",
            "My Doc",
            "--kind",
            "markdown",
            "--content",
            "# Hi",
        ];
        assert_eq!(ok_method(words), "artifact.add");
        assert_eq!(
            ok_params(words),
            json!({
                "workspaceId": "ws1",
                "title": "My Doc",
                "kind": "markdown",
                "content": "# Hi",
                "agentId": "self1"
            })
        );
    }

    #[test]
    fn artifact_add_sentinel_agent_omits_agent_id() {
        let params = ok_params(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "text",
            "--content",
            "x",
        ]);
        assert!(params.get("agentId").is_none(), "'-' must not set agentId");
    }

    #[test]
    fn artifact_add_carries_optional_filename() {
        let params = ok_params(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "code",
            "--content",
            "x",
            "--filename",
            "main.rs",
        ]);
        assert_eq!(params.get("filename"), Some(&json!("main.rs")));
    }

    #[test]
    fn artifact_add_missing_title_or_content_is_invalid() {
        assert!(is_invalid(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--kind",
            "text",
            "--content",
            "x"
        ]));
        assert!(is_invalid(&[
            "artifact", "add", "-", "ws1", "--title", "T", "--kind", "text"
        ]));
    }

    #[test]
    fn artifact_list_and_get_map_correctly() {
        assert_eq!(ok_method(&["artifact", "list", "ws1"]), "artifact.list");
        assert_eq!(
            ok_params(&["artifact", "list", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
        assert_eq!(ok_method(&["artifact", "get", "id1"]), "artifact.get");
        assert_eq!(
            ok_params(&["artifact", "get", "id1"]),
            json!({ "id": "id1" })
        );
    }

    #[test]
    fn artifact_unknown_sub_is_invalid() {
        assert!(is_invalid(&["artifact", "bogus", "ws1"]));
    }

    #[test]
    fn artifact_add_rejects_both_content_and_file_server_side() {
        // --file must be resolved client-side; the choke point rejects it even
        // when --content is also present (a raw cli.exec caller bypasses the
        // official client's preprocessing).
        assert!(is_invalid(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "text",
            "--content",
            "x",
            "--file",
            "/etc/passwd",
        ]));
    }

    #[test]
    fn artifact_add_rejects_bare_file_flag() {
        assert!(is_invalid(&[
            "artifact", "add", "-", "ws1", "--title", "T", "--kind", "text", "--file", "/x",
        ]));
    }

    #[test]
    fn artifact_add_rejects_duplicate_flag() {
        assert!(is_invalid(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "text",
            "--content",
            "a",
            "--content",
            "b",
        ]));
    }

    #[test]
    fn artifact_add_rejects_unknown_flag() {
        assert!(is_invalid(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "text",
            "--content",
            "x",
            "--bogus",
            "y",
        ]));
    }

    #[test]
    fn artifact_add_rejects_dangling_flag_value() {
        assert!(is_invalid(&[
            "artifact",
            "add",
            "-",
            "ws1",
            "--title",
            "T",
            "--kind",
            "text",
            "--content",
        ]));
    }

    // ── position / org (spec position-system §5) ──────────────────────────

    #[test]
    fn position_set_maps_value_flags() {
        // <ws> positional is present but NOT forwarded (spec §5.1 payload).
        assert_eq!(
            ok_method(&[
                "position",
                "set",
                "ws1",
                "a1",
                "--level",
                "senior",
                "--supervisor",
                "sup1"
            ]),
            "instance.setPosition"
        );
        assert_eq!(
            ok_params(&[
                "position",
                "set",
                "ws1",
                "a1",
                "--level",
                "senior",
                "--supervisor",
                "sup1"
            ]),
            json!({ "workspaceId": "ws1", "workspaceAgentId": "a1", "level": "senior", "supervisorAgentId": "sup1" })
        );
    }

    #[test]
    fn position_set_none_becomes_json_null_and_absent_key_is_omitted() {
        // --supervisor none clears (null); --level absent → key omitted (keep).
        assert_eq!(
            ok_params(&["position", "set", "ws1", "a1", "--supervisor", "none"]),
            json!({ "workspaceId": "ws1", "workspaceAgentId": "a1", "supervisorAgentId": null })
        );
        // --level none clears; no --supervisor → that key omitted.
        assert_eq!(
            ok_params(&["position", "set", "ws1", "a1", "--level", "none"]),
            json!({ "workspaceId": "ws1", "workspaceAgentId": "a1", "level": null })
        );
    }

    #[test]
    fn position_set_requires_at_least_one_flag() {
        assert!(is_invalid(&["position", "set", "ws1", "a1"]));
    }

    #[test]
    fn position_set_rejects_unknown_duplicate_and_dangling_flags() {
        assert!(is_invalid(&[
            "position", "set", "ws1", "a1", "--bogus", "x"
        ]));
        assert!(is_invalid(&[
            "position", "set", "ws1", "a1", "--level", "senior", "--level", "mid"
        ]));
        assert!(is_invalid(&["position", "set", "ws1", "a1", "--level"]));
    }

    #[test]
    fn position_unknown_sub_is_invalid() {
        assert!(is_invalid(&["position", "bogus", "ws1"]));
    }

    #[test]
    fn org_maps_to_instance_list() {
        assert_eq!(ok_method(&["org", "ws1"]), "instance.list");
        assert_eq!(ok_params(&["org", "ws1"]), json!({ "workspaceId": "ws1" }));
        assert!(is_invalid(&["org"]));
        assert!(is_invalid(&["org", "ws1", "extra"]));
    }

    // ── security tests ────────────────────────────────────────────────────

    /// Callers must NOT be able to reach `provider.upsert` via cli.exec.
    #[test]
    fn security_provider_upsert_is_rejected() {
        assert!(
            is_invalid(&["provider.upsert", "x"]),
            "router method names must not pass through as subcommands"
        );
    }

    /// Callers must NOT be able to invoke `cli.exec` recursively.
    #[test]
    fn security_cli_exec_is_rejected() {
        assert!(
            is_invalid(&["cli", "exec"]),
            "cli.exec must not be reachable as a subcommand"
        );
    }

    // ── async integration test through exec() ─────────────────────────────

    #[tokio::test]
    async fn exec_ws_list_returns_ok_array() {
        let state = AppState::for_tests().await;
        let result = exec(&state, json!({ "argv": ["ws", "list"] })).await;
        let value = result.expect("exec ws list should succeed");
        assert!(value.is_array(), "workspace.list returns an array: {value}");
    }

    #[tokio::test]
    async fn exec_memory_status_returns_ok_object() {
        let state = AppState::for_tests().await;
        let ws = crate::engine::repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");

        let result = exec(&state, json!({ "argv": ["memory", "status", &ws.id] })).await;
        let value = result.expect("exec memory status should succeed");
        assert!(
            value.is_object(),
            "memory.status returns an object: {value}"
        );
        assert_eq!(value["chunks"], json!(0));
    }

    /// End-to-end functional proof for the artifact CLI surface: the full
    /// `exec → map_argv → router → commands::artifact → DB` pipeline (only the
    /// socket JSON framing sits outside this). `add` then `list` then `get`
    /// must round-trip the same artifact.
    #[tokio::test]
    async fn exec_artifact_add_list_get_roundtrip() {
        let state = AppState::for_tests().await;
        let ws = crate::engine::repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");

        // add (agent slot "self1", inline --content)
        let added = exec(
            &state,
            json!({ "argv": [
                "artifact", "add", "self1", &ws.id,
                "--title", "Design Notes", "--kind", "markdown", "--content", "# Notes"
            ] }),
        )
        .await
        .expect("artifact add should succeed");
        assert_eq!(added["kind"], json!("markdown"));
        assert_eq!(added["agentId"], json!("self1"));
        let id = added["id"].as_str().expect("id present").to_string();

        // list
        let listed = exec(&state, json!({ "argv": ["artifact", "list", &ws.id] }))
            .await
            .expect("artifact list should succeed");
        assert_eq!(listed.as_array().expect("array").len(), 1);
        assert_eq!(listed[0]["id"], json!(id));

        // get
        let got = exec(&state, json!({ "argv": ["artifact", "get", &id] }))
            .await
            .expect("artifact get should succeed");
        assert_eq!(got["title"], json!("Design Notes"));
        assert_eq!(got["content"], json!("# Notes"));
    }

    /// Create a workspace agent instance and return its id (for the position
    /// exec test's supervisor chain).
    async fn mk_instance(state: &AppState, ws_id: &str, name: &str) -> String {
        use crate::engine::repo::agent_definition::{self, AgentDefinitionInput};
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "chat".into(),
                model: Some("m".into()),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def");
        crate::engine::repo::workspace_agent::instantiate(&state.db, ws_id, &def.id)
            .await
            .expect("instantiate")
            .id
    }

    /// End-to-end functional proof for `position set` through the full
    /// `exec → map_argv → router → instance::set_position → DB` pipeline: builds
    /// a 3-level chain, exercises tri-state keep/clear, and asserts every §3.5
    /// rejection (bad level, self-link, cross-workspace, cycle).
    #[tokio::test]
    async fn exec_position_set_validates_and_builds_a_chain() {
        let state = AppState::for_tests().await;
        let ws = crate::engine::repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("ws");
        let ws2 = crate::engine::repo::workspace::create(&state.db, "WS2", "/tmp/ws2", None)
            .await
            .expect("ws2");
        let lead = mk_instance(&state, &ws.id, "Lead").await;
        let sub = mk_instance(&state, &ws.id, "Sub").await;
        let imp = mk_instance(&state, &ws.id, "Impl").await;
        let foreign = mk_instance(&state, &ws2.id, "Foreign").await;

        // Happy: sub reports to lead, level senior — supervisorName resolves.
        let r = exec(
            &state,
            json!({ "argv":
            ["position","set",&ws.id,&sub,"--supervisor",&lead,"--level","senior"] }),
        )
        .await
        .expect("set sub");
        assert_eq!(r["level"], json!("senior"));
        assert_eq!(r["supervisorAgentId"], json!(lead));
        assert_eq!(r["supervisorName"], json!("Lead"));

        // Extend to a 3-level chain: impl -> sub -> lead.
        exec(
            &state,
            json!({ "argv": ["position","set",&ws.id,&imp,"--supervisor",&sub] }),
        )
        .await
        .expect("set impl");

        // Rejections (§3.5), while the full chain is in place:
        // cycle — lead reporting to impl closes impl->sub->lead->impl.
        assert!(
            exec(
                &state,
                json!({ "argv": ["position","set",&ws.id,&lead,"--supervisor",&imp] })
            )
            .await
            .is_err(),
            "cycle must be rejected"
        );
        // self-link.
        assert!(
            exec(
                &state,
                json!({ "argv": ["position","set",&ws.id,&imp,"--supervisor",&imp] })
            )
            .await
            .is_err(),
            "self-link must be rejected"
        );
        // cross-workspace supervisor.
        assert!(
            exec(
                &state,
                json!({ "argv": ["position","set",&ws.id,&imp,"--supervisor",&foreign] })
            )
            .await
            .is_err(),
            "cross-workspace supervisor must be rejected"
        );
        // bad level.
        assert!(
            exec(
                &state,
                json!({ "argv": ["position","set",&ws.id,&imp,"--level","wizard"] })
            )
            .await
            .is_err(),
            "unknown level must be rejected"
        );
        // wrong-workspace target (646ec73a): naming ws2 for an agent in ws.
        assert!(
            exec(
                &state,
                json!({ "argv": ["position","set",&ws2.id,&imp,"--level","senior"] })
            )
            .await
            .is_err(),
            "wrong-workspace target must be rejected"
        );

        // Tri-state KEEP: setting only --level leaves the supervisor intact.
        let r2 = exec(
            &state,
            json!({ "argv": ["position","set",&ws.id,&sub,"--level","principal"] }),
        )
        .await
        .expect("level only");
        assert_eq!(r2["level"], json!("principal"));
        assert_eq!(
            r2["supervisorAgentId"],
            json!(lead),
            "supervisor kept when --supervisor absent"
        );

        // Tri-state CLEAR: --supervisor none clears it, keeping the level.
        let r3 = exec(
            &state,
            json!({ "argv": ["position","set",&ws.id,&sub,"--supervisor","none"] }),
        )
        .await
        .expect("clear supervisor");
        assert!(
            r3.get("supervisorAgentId").is_none(),
            "supervisor cleared → key omitted"
        );
        assert_eq!(
            r3["level"],
            json!("principal"),
            "level kept when supervisor cleared"
        );
    }

    // ── browser ───────────────────────────────────────────────────────────

    // Agent browser verbs arrive at the engine with the caller id spliced in at
    // argv[2] (conclave-cli injects it from CONCLAVE_INSTANCE_ID). These tests
    // exercise that ENGINE-side wire form: `browser <verb> <callerId> <args…>`,
    // and assert the id re-attaches as the trusted `callerId` param.
    #[test]
    fn browser_open_and_goto_carry_url_and_caller_id() {
        assert_eq!(
            ok_method(&["browser", "open", "agentA", "example.com"]),
            "browser.open"
        );
        assert_eq!(
            ok_params(&["browser", "open", "agentA", "example.com"]),
            json!({ "url": "example.com", "callerId": "agentA" })
        );
        assert_eq!(
            ok_method(&["browser", "goto", "agentA", "https://x.test/"]),
            "browser.goto"
        );
        assert_eq!(
            ok_params(&["browser", "goto", "agentA", "https://x.test/"]),
            json!({ "url": "https://x.test/", "callerId": "agentA" })
        );
    }

    #[test]
    fn browser_open_requires_exactly_one_url_after_the_caller_id() {
        assert!(is_invalid(&["browser", "open", "agentA"])); // id but no url
        assert!(is_invalid(&["browser", "open", "agentA", "a", "b"])); // two urls
        assert!(is_invalid(&["browser", "goto", "agentA"]));
    }

    #[test]
    fn browser_status_carries_no_caller_id_close_does() {
        // status is a GLOBAL read — no id spliced, stays void.
        assert_eq!(ok_method(&["browser", "status"]), "browser.status");
        assert_eq!(ok_params(&["browser", "status"]), Value::Null);
        assert!(is_invalid(&["browser", "status", "extra"]));
        // close scopes to the caller's own tab → callerId at argv[2].
        assert_eq!(ok_method(&["browser", "close", "agentA"]), "browser.close");
        assert_eq!(
            ok_params(&["browser", "close", "agentA"]),
            json!({ "callerId": "agentA" })
        );
        assert!(is_invalid(&["browser", "close", "agentA", "extra"]));
    }

    #[test]
    fn browser_snapshot_optional_max_text_with_caller_id() {
        assert_eq!(
            ok_method(&["browser", "snapshot", "agentA"]),
            "browser.snapshot"
        );
        assert_eq!(
            ok_params(&["browser", "snapshot", "agentA"]),
            json!({ "callerId": "agentA" })
        );
        assert_eq!(
            ok_params(&["browser", "snapshot", "agentA", "--max-text", "500"]),
            json!({ "maxText": 500, "callerId": "agentA" })
        );
        // Non-integer and stray positional args are rejected.
        assert!(is_invalid(&["browser", "snapshot", "agentA", "--max-text", "lots"]));
        assert!(is_invalid(&["browser", "snapshot", "agentA", "junk"]));
    }

    #[test]
    fn browser_click_and_type_map_selector_and_text_with_caller_id() {
        assert_eq!(
            ok_params(&["browser", "click", "agentA", "#submit"]),
            json!({ "selector": "#submit", "callerId": "agentA" })
        );
        assert!(is_invalid(&["browser", "click", "agentA"]));
        // type joins the trailing words into one text payload.
        assert_eq!(
            ok_method(&["browser", "type", "agentA", "#q", "hello", "world"]),
            "browser.type"
        );
        assert_eq!(
            ok_params(&["browser", "type", "agentA", "#q", "hello", "world"]),
            json!({ "selector": "#q", "text": "hello world", "callerId": "agentA" })
        );
        assert!(is_invalid(&["browser", "type", "agentA", "#q"]));
    }

    #[test]
    fn browser_eval_joins_js_and_rejects_empty_with_caller_id() {
        assert_eq!(
            ok_params(&["browser", "eval", "agentA", "document.title"]),
            json!({ "js": "document.title", "callerId": "agentA" })
        );
        assert_eq!(
            ok_params(&["browser", "eval", "agentA", "1", "+", "2"]),
            json!({ "js": "1 + 2", "callerId": "agentA" })
        );
        assert!(is_invalid(&["browser", "eval", "agentA"]));
    }

    /// Anti-spoof: the caller id is taken POSITIONALLY from argv[2], never
    /// scavenged from a verb's free-text args. A crafted `eval` whose JS text
    /// itself looks like another agent id (or a `--caller-id` flag) cannot
    /// override the trusted id — the JS maps verbatim, the id stays argv[2].
    #[test]
    fn browser_caller_id_is_positional_not_from_free_text_args() {
        assert_eq!(
            ok_params(&["browser", "eval", "agentA", "other-agent-id", "--caller-id", "evil"]),
            json!({ "js": "other-agent-id --caller-id evil", "callerId": "agentA" })
        );
        assert_eq!(
            ok_params(&["browser", "type", "agentA", "#in", "--caller-id", "evil"]),
            json!({ "selector": "#in", "text": "--caller-id evil", "callerId": "agentA" })
        );
    }

    #[test]
    fn browser_screenshot_maps_with_defaults_and_flags_and_caller_id() {
        // path is positional; already absolute by the time map_argv sees it.
        assert_eq!(
            ok_method(&["browser", "screenshot", "agentA", "/ws/shot.png"]),
            "browser.screenshot"
        );
        assert_eq!(
            ok_params(&["browser", "screenshot", "agentA", "/ws/shot.png"]),
            json!({ "path": "/ws/shot.png", "callerId": "agentA" })
        );
        assert_eq!(
            ok_params(&[
                "browser",
                "screenshot",
                "agentA",
                "/ws/shot.png",
                "--width",
                "1440",
                "--height",
                "900"
            ]),
            json!({ "path": "/ws/shot.png", "width": 1440.0, "height": 900.0, "callerId": "agentA" })
        );
    }

    #[test]
    fn browser_screenshot_rejects_bad_dimension() {
        assert!(is_invalid(&[
            "browser",
            "screenshot",
            "agentA",
            "/ws/s.png",
            "--width",
            "abc"
        ]));
    }

    #[test]
    fn browser_unknown_subcommand_is_invalid() {
        assert!(is_invalid(&["browser"]));
        assert!(is_invalid(&["browser", "frobnicate"]));
    }

    // ── code ──────────────────────────────────────────────────────────────

    #[test]
    fn code_find_maps_to_router_method() {
        let (method, params) = map_argv(&argv(&[
            "code", "find", "greet", "--exact", "--path", "/tmp/x",
        ]))
        .unwrap();
        assert_eq!(method, "code.find");
        assert_eq!(
            params,
            serde_json::json!({"name": "greet", "exact": true, "path": "/tmp/x"})
        );
    }

    #[test]
    fn code_rename_requires_old_and_new() {
        let err = map_argv(&argv(&["code", "rename", "onlyone", "--path", "/tmp/x"])).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[test]
    fn code_rejects_unknown_verb() {
        let err = map_argv(&argv(&["code", "dance", "--path", "/tmp/x"])).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    // ── proxy ─────────────────────────────────────────────────────────────

    #[test]
    fn proxy_verbs_map_to_allowlisted_router_methods() {
        assert_eq!(ok_method(&["proxy", "status"]), "proxy.status");
        assert_eq!(ok_params(&["proxy", "status"]), Value::Null);
        assert_eq!(ok_method(&["proxy", "mode", "rewrite"]), "proxy.mode");
        assert_eq!(
            ok_params(&["proxy", "mode", "rewrite"]),
            json!({ "mode": "rewrite" })
        );
        assert_eq!(ok_method(&["proxy", "report"]), "proxy.report");
        assert_eq!(
            ok_params(&["proxy", "report", "--since-hours", "48"]),
            json!({ "sinceHours": 48 })
        );
        assert_eq!(ok_method(&["proxy", "threshold", "0.25"]), "proxy.threshold");
        assert_eq!(
            ok_params(&["proxy", "threshold", "0.25"]),
            json!({ "ratio": 0.25 })
        );
    }

    #[test]
    fn proxy_rejects_unknown_or_malformed_verbs() {
        assert!(is_invalid(&["proxy", "nuke"]));
        assert!(is_invalid(&["proxy", "mode", "turbo"]));
        assert!(is_invalid(&[
            "proxy",
            "report",
            "--since-hours",
            "yesterday"
        ]));
        assert!(is_invalid(&["proxy", "report", "--since-hours", "-1"]));
        assert!(is_invalid(&["proxy", "threshold", "half"]));
        assert!(is_invalid(&["proxy", "threshold"]));
    }

    #[test]
    fn code_all_verbs_map_flags_to_camel_case_params() {
        assert_eq!(
            ok_params(&["code", "stats", "--json", "--path", "/x"]),
            json!({"path": "/x"})
        );
        assert_eq!(
            ok_params(&["code", "files", "--limit", "4", "--path", "/x"]),
            json!({"path": "/x", "limit": 4})
        );
        assert_eq!(
            ok_params(&[
                "code",
                "symbols",
                "src/lib.rs",
                "--all",
                "--kind",
                "fn,struct",
                "--limit",
                "8",
                "--path",
                "/x"
            ]),
            json!({
                "path": "/x",
                "target": "src/lib.rs",
                "all": true,
                "kind": ["fn", "struct"],
                "limit": 8,
            })
        );
        assert_eq!(
            ok_params(&["code", "callers", "greet", "--depth", "2", "--path", "/x"]),
            json!({"path": "/x", "name": "greet", "depth": 2})
        );
        assert_eq!(
            ok_params(&["code", "callees", "greet", "--path", "/x"]),
            json!({"path": "/x", "name": "greet"})
        );
        assert_eq!(
            ok_params(&["code", "refs", "greet", "--limit", "3", "--path", "/x"]),
            json!({"path": "/x", "name": "greet", "limit": 3})
        );
        assert_eq!(
            ok_params(&["code", "impact", "greet", "--path", "/x"]),
            json!({"path": "/x", "name": "greet"})
        );
        assert_eq!(
            ok_params(&[
                "code",
                "rename",
                "old",
                "new",
                "--apply",
                "--lang",
                "rust",
                "--anchor",
                "src/lib.rs:1",
                "--path",
                "/x"
            ]),
            json!({
                "path": "/x",
                "old": "old",
                "new": "new",
                "apply": true,
                "lang": "rust",
                "anchor": "src/lib.rs:1",
            })
        );
        assert_eq!(
            ok_params(&[
                "code",
                "rewrite",
                "--pattern",
                "old($A)",
                "--rewrite",
                "new($A)",
                "--apply",
                "--lang",
                "rust",
                "--path",
                "/x"
            ]),
            json!({
                "path": "/x",
                "pattern": "old($A)",
                "rewrite": "new($A)",
                "apply": true,
                "lang": "rust",
            })
        );
        assert_eq!(ok_method(&["code", "tree", "--path", "/x"]), "code.tree");
    }

    #[test]
    fn code_rejects_missing_path_bad_numbers_and_stray_words() {
        assert!(is_invalid(&["code", "stats"]));
        assert!(is_invalid(&[
            "code", "files", "--limit", "many", "--path", "/x"
        ]));
        assert!(is_invalid(&[
            "code", "callers", "greet", "--depth", "-1", "--path", "/x"
        ]));
        assert!(is_invalid(&[
            "code",
            "rewrite",
            "--pattern",
            "old($A)",
            "--path",
            "/x"
        ]));
        assert!(is_invalid(&["code", "tree", "extra", "--path", "/x"]));
    }
}
