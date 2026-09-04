# AI Agent & Team Drafter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, lead) · authority: in-loop

**Goal:** Let the user type a brief and have a CLI model (claude/codex in print mode) draft one agent definition or a whole workspace team, reviewed and applied through the existing commands.

**Architecture:** A Rust one-shot runner (`runtime/cli_oneshot.rs`) launches `claude -p --json-schema` / `codex exec --output-schema` inside the user's login shell with the drafter definition's env, returns structured JSON; `commands/draft.rs` builds the catalogue + prompt, validates the draft against real ids, and exposes `draft.agents`. The frontend `AgentDrafter.tsx` overlay shows an editable preview; agent mode opens Builder pre-filled, team mode applies via `role.save → agentDef.save → agentDef.addToWorkspace → instance.setPosition`.

**Tech Stack:** Tauri v2, Rust (tokio process, serde_json, tempfile), React 19 + TypeScript strict, Tailwind.

**Spec:** `docs/superpowers/specs/2026-09-04-ai-agent-team-drafter-design.md` — read it first; decisions D1–D11 are final.

## Global Constraints

- No DB migration. No new bus event. No new write command — team apply is frontend orchestration over existing commands (spec D6).
- `tempfile` is a normal `[dependencies]` entry as of main 6192ed8 (it was dev-only; found by Dew, challenge df8ae20b). Lane A merges main into its worktree before Task A2 — no lane edits `src-tauri/Cargo.toml`.
- Rust tests never spawn a real `claude`/`codex` (spec D7); the only real run is the recorded manual gate in Task A5.
- Rust gates per task, run in `src-tauri`: `cargo test -p conclave` (or the crate name in `src-tauri/Cargo.toml` `[package] name`), `cargo clippy --all-targets -- -D warnings`, and `rustfmt --check <your files>` ONLY (never bare `cargo fmt` — main has 17 files of fmt drift, bb `warning:fmt-drift-main`).
- Frontend gates: `pnpm exec tsc --noEmit`, `pnpm build`, `pnpm uishot <view>` with the PNG opened and inspected (CLAUDE.md UI Pixel Gate). A fresh lane worktree needs `pnpm install` once.
- All UI copy English. Fixture handlers use fixed literal timestamps and fixed literal data.
- Serde: every new Rust struct crossing IPC is `#[serde(rename_all = "camelCase")]`; TS types mirror them field-for-field.
- Commit per task with an explicit pathspec (`git commit -- <paths>`); never sweep unrelated files.
- Lanes: A (Rust) and C (frontend) share only the `draft.agents` wire contract defined in Task A3 / Task C2 — copy it verbatim, do not "improve" it in one lane. B (design canon) precedes C.

---

## File structure

| File | Responsibility | Lane |
|---|---|---|
| `src-tauri/src/engine/runtime/launch_common.rs` (new) | `shell_quote`, `effective_claude_model`, `agent_env_overrides`, `prefix_conclave_path` — pure launch helpers shared by `instance::spawn` and the one-shot runner | A |
| `src-tauri/src/engine/runtime/cli_oneshot.rs` (new) | `OneshotSpec`, `Oneshot {Live, Mock}`, launch-string builders, envelope parsing, timeout/kill | A |
| `src-tauri/src/engine/commands/draft.rs` (new) | wire types, catalogue, JSON schema, validator, `run` handler | A |
| `src-tauri/src/engine/commands/draft_prompt.rs` (new) | `build_prompt` and its const fragments | A |
| `src-tauri/src/engine/commands/instance.rs`, `commands/fusion.rs`, `commands/mod.rs`, `runtime/mod.rs`, `router.rs` | small edits: use hoisted helpers, `pub(crate) strip_code_fences`, register modules + route | A |
| `src/lib/modelCatalogue.ts` (new) | `CLAUDE_MODELS`, `CODEX_MODELS`, `COLOR_SWATCHES` lifted from Builder | C |
| `src/ipc/types.ts`, `src/ipc/commands.ts`, `src/ipc/index.ts` | `DraftAgent`/`DraftPosition`/`DraftResponse`, `draft.agents` command | C |
| `src/fixtures/scenarios/data.ts`, `default.ts`, `empty.ts` | fixed-literal `draft.agents` handler | C |
| `src/lib/applyTeamDraft.ts` (new) | `topoOrder`, `applyTeamDraft` executor with progress callback | C |
| `src/components/AgentDrafter.tsx` (new) | overlay: brief, drafter picker, waiting/error, preview table, apply progress | C |
| `src/components/AppShell.tsx`, `Builder.tsx`, `Library.tsx`, `Roster.tsx` | wiring, `isEditing` fix + `draftedBy` chip, entry buttons | C |

---

# Lane A — Rust one-shot runner and `draft.agents`

### Task A1: Hoist launch helpers into `runtime/launch_common.rs`

**Files:**
- Create: `src-tauri/src/engine/runtime/launch_common.rs`
- Modify: `src-tauri/src/engine/runtime/mod.rs` (add `pub mod launch_common;` in the alphabetical module list)
- Modify: `src-tauri/src/engine/commands/instance.rs` (remove the two private fns at ~:48-61, replace the env block at ~:867-899 and the PATH prefix at ~:874-880)

**Interfaces:**
- Produces:
  - `pub fn shell_quote(s: &str) -> String`
  - `pub fn effective_claude_model(model: &str, context_window: Option<&str>) -> String`
  - `pub fn agent_env_overrides(def: &AgentDefRow) -> Vec<(String, String)>` — `custom_env` JSON object string values, then Keychain secrets named in `secret_env_keys` (account `agent_env:{def.id}:{name}`, missing ones skipped).
  - `pub fn prefix_conclave_path(launch: String) -> String` — prepends the `export PATH=<shim dir>:"$PATH"; ` prefix exactly as `instance::spawn` does today when `agentctx::ensure_conclave_shim()` returns a path; identity otherwise.

- [ ] **Step 1: Write the failing tests** in the new file

```rust
// src-tauri/src/engine/runtime/launch_common.rs
//! Launch-string and env helpers shared by the PTY spawn path
//! (`commands::instance::spawn`) and the one-shot print-mode runner
//! (`runtime::cli_oneshot`). Pure: no I/O except the Keychain read in
//! `agent_env_overrides`, which tests avoid by leaving `secret_env_keys` None.

use crate::engine::repo::agent_definition::AgentDefRow;

/// Single-quote a value so the shell doesn't glob it; POSIX-escape embedded
/// quotes so the value can't break out of the launch command.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn effective_claude_model(model: &str, context_window: Option<&str>) -> String {
    if context_window == Some("1m") {
        format!("{model}[1m]")
    } else {
        model.to_string()
    }
}

/// Non-secret env from the definition's `custom_env` JSON object, then secret
/// values fetched back from the Keychain by the names in `secret_env_keys`.
pub fn agent_env_overrides(def: &AgentDefRow) -> Vec<(String, String)> {
    let mut extra_env: Vec<(String, String)> = Vec::new();
    if let Some(text) = def.custom_env.as_deref() {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    extra_env.push((k, s.to_owned()));
                }
            }
        }
    }
    if let Some(text) = def.secret_env_keys.as_deref() {
        if let Ok(serde_json::Value::Array(names)) = serde_json::from_str::<serde_json::Value>(text) {
            for name in names.iter().filter_map(|n| n.as_str()) {
                let account = format!("agent_env:{}:{}", def.id, name);
                if let Ok(Some(val)) = crate::engine::secrets::get_key(&account) {
                    extra_env.push((name.to_owned(), val));
                }
            }
        }
    }
    extra_env
}

/// Prepend the bundled `conclave` shim dir to PATH for the child shell.
/// Copy the EXACT format string from `commands/instance.rs` (grep `export PATH=`)
/// so the PTY path's behaviour is unchanged after it calls this.
pub fn prefix_conclave_path(launch: String) -> String {
    match crate::engine::agentctx::ensure_conclave_shim() {
        Some(bin) => format!(
            "export PATH={}:\"$PATH\"; {}",
            shell_quote(&bin.to_string_lossy()),
            launch
        ),
        None => launch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_with_env(custom_env: Option<&str>) -> AgentDefRow {
        // Build via serde so the test does not depend on AgentDefRow's field count.
        let mut v = serde_json::json!({
            "id": "def-1", "name": "X", "type": "cli", "harnessMode": "own",
            "createdAt": "2026-09-04T00:00:00Z"
        });
        if let Some(e) = custom_env {
            v["customEnv"] = serde_json::Value::String(e.to_string());
        }
        serde_json::from_value(v).expect("AgentDefRow from json")
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("plain"), "'plain'");
    }

    #[test]
    fn effective_model_adds_1m_suffix_only_for_1m() {
        assert_eq!(effective_claude_model("claude-opus-4-8", Some("1m")), "claude-opus-4-8[1m]");
        assert_eq!(effective_claude_model("claude-opus-4-8", Some("200k")), "claude-opus-4-8");
        assert_eq!(effective_claude_model("claude-opus-4-8", None), "claude-opus-4-8");
    }

    #[test]
    fn env_overrides_reads_string_values_only() {
        let def = def_with_env(Some(r#"{"A":"1","B":2,"C":"x"}"#));
        let env = agent_env_overrides(&def);
        assert_eq!(env, vec![("A".into(), "1".into()), ("C".into(), "x".into())]);
    }

    #[test]
    fn env_overrides_empty_when_no_custom_env() {
        assert!(agent_env_overrides(&def_with_env(None)).is_empty());
    }
}
```

If `AgentDefRow` does not derive `Deserialize` (check `repo/agent_definition.rs:60`), build the row with `AgentDefRow { id: "def-1".into(), custom_env: ..., ..Default::default() }` instead and add `#[derive(Default)]` to the row only if every field is `Default`-able; otherwise construct it fully. Do not change the row's serde attributes.

- [ ] **Step 2: Register the module and run the tests**

Add `pub mod launch_common;` to `src-tauri/src/engine/runtime/mod.rs`.
Run: `cd src-tauri && cargo test launch_common -- --nocapture`
Expected: 4 passed.

- [ ] **Step 3: Switch `instance.rs` to the shared helpers**

- Delete the private `fn shell_quote` and `fn effective_claude_model` (around :48-61) and add near the top imports:
  `use crate::engine::runtime::launch_common::{agent_env_overrides, effective_claude_model, prefix_conclave_path, shell_quote};`
- Replace the custom_env + secret_env_keys loops (the block that begins `if let Some(text) = def.custom_env.as_deref()` through the end of the secrets loop, ~:874-899) with `extra_env.extend(agent_env_overrides(&def));` — keep the two `CONCLAVE_WORKSPACE_ID` / `CONCLAVE_INSTANCE_ID` pushes above it.
- Replace the `export PATH=` block (~:874-880) with `launch = prefix_conclave_path(launch);` — after Step 1 you copied its format string, so this is a pure move.

- [ ] **Step 4: Run the full gate**

Run: `cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && rustfmt --check src/engine/runtime/launch_common.rs src/engine/runtime/mod.rs src/engine/commands/instance.rs`
Expected: all green; instance spawn tests unchanged.

- [ ] **Step 5: Commit**

```bash
git commit -m "refactor(launch): hoist shell_quote/effective_claude_model/env overrides into runtime::launch_common" -- src-tauri/src/engine/runtime/launch_common.rs src-tauri/src/engine/runtime/mod.rs src-tauri/src/engine/commands/instance.rs
```

### Task A2: `runtime/cli_oneshot.rs` — launch strings, envelope parsing, Live/Mock runner

**Files:**
- Create: `src-tauri/src/engine/runtime/cli_oneshot.rs`
- Modify: `src-tauri/src/engine/runtime/mod.rs` (`pub mod cli_oneshot;`)
- Modify: `src-tauri/src/engine/commands/fusion.rs:168` — change `fn strip_code_fences` to `pub(crate) fn strip_code_fences` (one-word change, no behaviour change)

**Interfaces:**
- Consumes: `launch_common::{shell_quote, effective_claude_model, prefix_conclave_path}` (A1), `fusion::strip_code_fences`.
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliKind { ClaudeCode, Codex }
impl CliKind { pub fn parse(s: &str) -> Option<CliKind> /* "claude-code" | "codex" */ }

pub struct OneshotSpec {
    pub cli_kind: CliKind,
    pub model: Option<String>,          // already passed through effective_claude_model by the caller for claude
    pub prompt: String,                 // written to the child's stdin
    pub json_schema: serde_json::Value,
    pub extra_env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
    pub timeout: std::time::Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum OneshotError {
    #[error("the drafter did not answer in {0} s")] Timeout(u64),
    #[error("{cli} exited with code {code}: {stderr_tail}")] Exit { cli: &'static str, code: i32, stderr_tail: String },
    #[error("the drafter reported an error: {0}")] Model(String),
    #[error("could not parse the drafter's output: {0}")] Parse(String),
    #[error("could not launch {0}")] Spawn(String),
}

pub enum Oneshot { Live, #[cfg(test)] Mock(Result<serde_json::Value, String>) }
impl Oneshot { pub async fn run(&self, spec: &OneshotSpec) -> Result<serde_json::Value, OneshotError> }

pub fn claude_launch(model: Option<&str>, schema: &serde_json::Value) -> String;
pub fn codex_launch(model: Option<&str>, schema_path: &std::path::Path, out_path: &std::path::Path) -> String;
pub fn extract_last_json_object(stdout: &str) -> Option<serde_json::Value>;
pub fn claude_structured_result(envelope: &serde_json::Value) -> Result<serde_json::Value, OneshotError>;
pub fn parse_codex_last_message(text: &str) -> Result<serde_json::Value, OneshotError>;
```

- [ ] **Step 1: Write the failing tests** (bottom of the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_launch_has_print_schema_no_session_and_model() {
        let s = claude_launch(Some("claude-sonnet-5"), &json!({"type":"object"}));
        assert_eq!(
            s,
            "claude -p --output-format json --json-schema '{\"type\":\"object\"}' --no-session-persistence --tools '' --model 'claude-sonnet-5'"
        );
    }

    #[test]
    fn claude_launch_omits_model_when_none() {
        let s = claude_launch(None, &json!({}));
        assert!(s.ends_with("--tools ''"));
        assert!(!s.contains("--model"));
    }

    #[test]
    fn codex_launch_uses_exec_json_schema_and_last_message_file() {
        let s = codex_launch(Some("gpt-5.5"), std::path::Path::new("/t/s.json"), std::path::Path::new("/t/o.json"));
        assert_eq!(
            s,
            "codex exec --json --ephemeral --skip-git-repo-check --output-schema '/t/s.json' -o '/t/o.json' -m 'gpt-5.5' -"
        );
    }

    #[test]
    fn extract_last_json_object_skips_shell_banner_lines() {
        let out = "Welcome to zsh\n{\"type\":\"system\"}\nnoise\n{\"type\":\"result\",\"n\":1}\n";
        let v = extract_last_json_object(out).unwrap();
        assert_eq!(v["type"], "result");
        assert_eq!(v["n"], 1);
    }

    #[test]
    fn extract_last_json_object_none_when_no_json() {
        assert!(extract_last_json_object("nothing here").is_none());
    }

    #[test]
    fn claude_structured_result_returns_structured_output_on_success() {
        let env = json!({"type":"result","subtype":"success","is_error":false,"structured_output":{"a":1},"result":"{\"a\":1}"});
        assert_eq!(claude_structured_result(&env).unwrap(), json!({"a":1}));
    }

    #[test]
    fn claude_structured_result_falls_back_to_result_text() {
        let env = json!({"type":"result","subtype":"success","is_error":false,"result":"```json\n{\"a\":2}\n```"});
        assert_eq!(claude_structured_result(&env).unwrap(), json!({"a":2}));
    }

    #[test]
    fn claude_structured_result_errors_on_is_error() {
        let env = json!({"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"});
        match claude_structured_result(&env) {
            Err(OneshotError::Model(m)) => assert!(m.contains("boom")),
            other => panic!("expected Model error, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_last_message_strips_fences() {
        assert_eq!(parse_codex_last_message("```json\n{\"k\":true}\n```").unwrap(), json!({"k":true}));
    }

    #[tokio::test]
    async fn mock_runner_returns_canned_value() {
        let spec = OneshotSpec {
            cli_kind: CliKind::ClaudeCode, model: None, prompt: "p".into(), json_schema: json!({}),
            extra_env: vec![], cwd: std::env::temp_dir(), timeout: std::time::Duration::from_secs(1),
        };
        let v = Oneshot::Mock(Ok(json!({"ok":1}))).run(&spec).await.unwrap();
        assert_eq!(v, json!({"ok":1}));
        let e = Oneshot::Mock(Err("nope".into())).run(&spec).await.unwrap_err();
        assert!(matches!(e, OneshotError::Model(_)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test cli_oneshot`
Expected: compile error — module/functions missing.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/engine/runtime/cli_oneshot.rs
//! One-shot, non-PTY invocation of a CLI harness in print mode
//! (`claude -p --json-schema …` / `codex exec --output-schema …`). Used by
//! `commands::draft` (spec D1). Launched through the user's login shell with the
//! same PATH prefix and env overrides as `instance::spawn`, but with no PTY, no
//! session row and no instance id. Tests never spawn a binary (`Mock`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::launch_common::{prefix_conclave_path, shell_quote};
use crate::engine::commands::fusion::strip_code_fences;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliKind { ClaudeCode, Codex }

impl CliKind {
    pub fn parse(s: &str) -> Option<CliKind> {
        match s { "claude-code" => Some(CliKind::ClaudeCode), "codex" => Some(CliKind::Codex), _ => None }
    }
    fn bin(self) -> &'static str {
        match self { CliKind::ClaudeCode => "claude", CliKind::Codex => "codex" }
    }
}

pub struct OneshotSpec {
    pub cli_kind: CliKind,
    pub model: Option<String>,
    pub prompt: String,
    pub json_schema: Value,
    pub extra_env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum OneshotError {
    #[error("the drafter did not answer in {0} s")]
    Timeout(u64),
    #[error("{cli} exited with code {code}: {stderr_tail}")]
    Exit { cli: &'static str, code: i32, stderr_tail: String },
    #[error("the drafter reported an error: {0}")]
    Model(String),
    #[error("could not parse the drafter's output: {0}")]
    Parse(String),
    #[error("could not launch {0}")]
    Spawn(String),
}

pub fn claude_launch(model: Option<&str>, schema: &Value) -> String {
    let mut s = format!(
        "claude -p --output-format json --json-schema {} --no-session-persistence --tools ''",
        shell_quote(&schema.to_string())
    );
    if let Some(m) = model {
        s.push_str(&format!(" --model {}", shell_quote(m)));
    }
    s
}

pub fn codex_launch(model: Option<&str>, schema_path: &Path, out_path: &Path) -> String {
    let mut s = format!(
        "codex exec --json --ephemeral --skip-git-repo-check --output-schema {} -o {}",
        shell_quote(&schema_path.to_string_lossy()),
        shell_quote(&out_path.to_string_lossy())
    );
    if let Some(m) = model {
        s.push_str(&format!(" -m {}", shell_quote(m)));
    }
    s.push_str(" -"); // prompt on stdin
    s
}

/// The LAST line of stdout that parses as a JSON object. An interactive login
/// shell may print banners before the CLI's envelope (spec R1).
pub fn extract_last_json_object(stdout: &str) -> Option<Value> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| serde_json::from_str::<Value>(l).ok().filter(Value::is_object))
}

pub fn claude_structured_result(envelope: &Value) -> Result<Value, OneshotError> {
    let is_error = envelope["is_error"].as_bool().unwrap_or(false);
    let subtype = envelope["subtype"].as_str().unwrap_or("");
    let result_text = envelope["result"].as_str().unwrap_or("").to_string();
    if is_error || subtype != "success" {
        return Err(OneshotError::Model(if result_text.is_empty() { subtype.to_string() } else { result_text }));
    }
    if let Some(v) = envelope.get("structured_output").filter(|v| v.is_object()) {
        return Ok(v.clone());
    }
    serde_json::from_str::<Value>(strip_code_fences(&result_text))
        .map_err(|e| OneshotError::Parse(format!("{e}; result text: {}", result_text.chars().take(300).collect::<String>())))
}

pub fn parse_codex_last_message(text: &str) -> Result<Value, OneshotError> {
    serde_json::from_str::<Value>(strip_code_fences(text))
        .map_err(|e| OneshotError::Parse(format!("{e}; last message: {}", text.chars().take(300).collect::<String>())))
}

pub enum Oneshot {
    Live,
    #[cfg(test)]
    Mock(Result<Value, String>),
}

impl Oneshot {
    pub async fn run(&self, spec: &OneshotSpec) -> Result<Value, OneshotError> {
        match self {
            Oneshot::Live => run_live(spec).await,
            #[cfg(test)]
            Oneshot::Mock(r) => r.clone().map_err(OneshotError::Model),
        }
    }
}

async fn run_live(spec: &OneshotSpec) -> Result<Value, OneshotError> {
    // Temp dir outlives the child: codex reads the schema from it and writes
    // its last message into it.
    let tmp = tempfile::tempdir().map_err(|e| OneshotError::Spawn(e.to_string()))?;
    let schema_path = tmp.path().join("schema.json");
    let out_path = tmp.path().join("last.json");
    let launch = match spec.cli_kind {
        CliKind::ClaudeCode => claude_launch(spec.model.as_deref(), &spec.json_schema),
        CliKind::Codex => {
            std::fs::write(&schema_path, spec.json_schema.to_string())
                .map_err(|e| OneshotError::Spawn(e.to_string()))?;
            codex_launch(spec.model.as_deref(), &schema_path, &out_path)
        }
    };
    let launch = prefix_conclave_path(launch);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.args(["-l", "-i", "-c", &launch])
        .current_dir(&spec.cwd)
        .env("CONCLAVE_DRAFT", "1")
        .envs(spec.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| OneshotError::Spawn(format!("{shell} -c {launch}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(spec.prompt.as_bytes()).await.map_err(|e| OneshotError::Spawn(e.to_string()))?;
        // Drop closes stdin so the CLI sees EOF and starts.
    }
    let output = match tokio::time::timeout(spec.timeout, child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| OneshotError::Spawn(e.to_string()))?,
        Err(_) => return Err(OneshotError::Timeout(spec.timeout.as_secs())),
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let tail_start = stderr.len().saturating_sub(2048);
        return Err(OneshotError::Exit {
            cli: spec.cli_kind.bin(),
            code: output.status.code().unwrap_or(-1),
            stderr_tail: stderr[tail_start..].to_string(),
        });
    }
    match spec.cli_kind {
        CliKind::ClaudeCode => {
            let envelope = extract_last_json_object(&stdout)
                .ok_or_else(|| OneshotError::Parse(format!("no JSON envelope on stdout; tail: {}", stdout.chars().rev().take(300).collect::<String>().chars().rev().collect::<String>())))?;
            claude_structured_result(&envelope)
        }
        CliKind::Codex => {
            let text = std::fs::read_to_string(&out_path)
                .map_err(|e| OneshotError::Parse(format!("codex wrote no last message: {e}")))?;
            parse_codex_last_message(&text)
        }
    }
}
```

Register `pub mod cli_oneshot;` in `runtime/mod.rs`; make `strip_code_fences` `pub(crate)` in `fusion.rs`. `thiserror` is already a dependency (check `Cargo.toml`; `error.rs` uses it) — if not, use a manual `Display` impl instead of adding a crate.

- [ ] **Step 4: Run the gate**

Run: `cd src-tauri && cargo test cli_oneshot && cargo clippy --all-targets -- -D warnings && rustfmt --check src/engine/runtime/cli_oneshot.rs src/engine/runtime/mod.rs src/engine/commands/fusion.rs`
Expected: 10 passed, clippy clean.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oneshot): non-PTY print-mode runner for claude/codex with structured JSON output" -- src-tauri/src/engine/runtime/cli_oneshot.rs src-tauri/src/engine/runtime/mod.rs src-tauri/src/engine/commands/fusion.rs
```

### Task A3: `commands/draft.rs` — wire types, catalogue, schema, validator

**Files:**
- Create: `src-tauri/src/engine/commands/draft.rs`
- Modify: `src-tauri/src/engine/commands/mod.rs` (`pub mod draft;` and `pub mod draft_prompt;` — the latter is created in A4; add it there)

**Interfaces:**
- Consumes: `repo::role::{list_builtin, list}` (`RoleRow {id,name,description,skill_ids,kind}`), `repo::skill::{list_builtin, list}` (`SkillRow {id,name,description,kind,mandatory,…}`), `repo::agent_definition::{list, get}` (`AgentDefRow`), `repo::workspace_agent::list_by_workspace` — open `repo/workspace_agent.rs:33-60` and `:104-123` to confirm the row's `level` / `supervisor_agent_id` field names before use.
- Produces (THE wire contract; Lane C copies it verbatim):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftMode { Agent, Team }

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest { pub mode: DraftMode, pub brief: String, pub drafter_def_id: String, pub workspace_id: Option<String> }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftNewRole { pub name: String, pub description: String, pub skill_ids: Vec<String> }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAgent {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub existing_agent_def_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cli_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub role_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub new_role: Option<DraftNewRole>,
    #[serde(default)] pub skill_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub default_level: Option<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftPosition { pub key: String, pub level: String, #[serde(default)] pub supervisor_key: Option<String> }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrafterInfo { pub def_id: String, pub cli_kind: String, pub model: String }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftResponse { pub agents: Vec<DraftAgent>, pub positions: Vec<DraftPosition>, #[serde(default)] pub notes: String, pub drafter: DrafterInfo }

pub const CLAUDE_MODELS: &[&str] = &["claude-fable-5-1", "claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5", "claude-opus-4-8"]; // human request 2026-09-04: add the Claude 5 family (Fable 5.1, Opus 5, Sonnet 5, Haiku 4.5) shown in Claude Code's picker; keep opus-4-8 for existing rows
pub const CODEX_MODELS: &[&str] = &["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex-spark"];
pub const COLOR_SWATCHES: &[&str] = &[/* copy ALL hex values from src/components/Builder.tsx:41-47 verbatim */];
pub const LEVELS: &[&str] = &["junior", "mid", "senior", "principal"];
pub const MAX_TEAM_SIZE: usize = 12;
pub const BRIEF_MAX_CHARS: usize = 4000;

pub struct RosterLine { pub workspace_agent_id: String, pub name: String, pub role_name: Option<String>, pub level: Option<String>, pub supervisor_name: Option<String> }
pub struct ExistingDef { pub id: String, pub name: String, pub role_name: Option<String>, pub cli_kind: Option<String>, pub model: Option<String> }
pub struct Catalogue { pub roles: Vec<RoleRow>, pub skills: Vec<SkillRow> /* mandatory == false only */, pub existing: Vec<ExistingDef>, pub roster: Vec<RosterLine> }

pub async fn build_catalogue(db: &SqlitePool, workspace_id: Option<&str>) -> Result<Catalogue, AppError>;
pub fn draft_schema(mode: DraftMode) -> serde_json::Value;   // JSON Schema handed to --json-schema / --output-schema
pub fn validate_draft(draft: &DraftResponse, mode: DraftMode, cat: &Catalogue) -> Result<(), String>; // Err("draft.<field>: <reason>")
```

- [ ] **Step 1: Write the failing validator tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalogue {
        Catalogue {
            roles: vec![RoleRow { id: "lead".into(), name: "Lead".into(), description: "d".into(), skill_ids: vec!["leadership".into()], kind: "builtin".into() },
                        RoleRow { id: "implementer".into(), name: "Implementer".into(), description: "d".into(), skill_ids: vec!["implementer".into()], kind: "builtin".into() }],
            skills: vec![skill("leadership"), skill("implementer"), skill("agent-loop")],
            existing: vec![ExistingDef { id: "def-existing".into(), name: "Dew".into(), role_name: Some("Implementer".into()), cli_kind: Some("claude-code".into()), model: Some("claude-opus-4-8".into()) }],
            roster: vec![],
        }
    }
    fn skill(id: &str) -> SkillRow {
        serde_json::from_value(serde_json::json!({"id": id, "name": id, "content": "", "kind": "builtin", "mandatory": false})).unwrap()
    }
    fn agent(key: &str) -> DraftAgent {
        DraftAgent { key: key.into(), existing_agent_def_id: None, name: Some(format!("A-{key}")), color: Some(COLOR_SWATCHES[0].into()),
            cli_kind: Some("claude-code".into()), model: Some("claude-sonnet-5".into()), role_id: Some("implementer".into()), new_role: None,
            skill_ids: vec!["agent-loop".into()], default_level: Some("senior".into()), rationale: "r".into() }
    }
    fn resp(agents: Vec<DraftAgent>, positions: Vec<DraftPosition>) -> DraftResponse {
        DraftResponse { agents, positions, notes: String::new(), drafter: DrafterInfo { def_id: "d".into(), cli_kind: "claude-code".into(), model: "m".into() } }
    }
    fn pos(key: &str, sup: Option<&str>) -> DraftPosition { DraftPosition { key: key.into(), level: "senior".into(), supervisor_key: sup.map(String::from) } }

    #[test] fn agent_mode_requires_exactly_one_agent_and_no_positions() {
        assert!(validate_draft(&resp(vec![agent("a")], vec![]), DraftMode::Agent, &cat()).is_ok());
        assert!(validate_draft(&resp(vec![agent("a"), agent("b")], vec![]), DraftMode::Agent, &cat()).unwrap_err().starts_with("draft.agents"));
        assert!(validate_draft(&resp(vec![agent("a")], vec![pos("a", None)]), DraftMode::Agent, &cat()).unwrap_err().starts_with("draft.positions"));
    }
    #[test] fn team_mode_rejects_duplicate_keys_unknown_supervisor_and_cycles() {
        let c = cat();
        assert!(validate_draft(&resp(vec![agent("a"), agent("a")], vec![pos("a", None)]), DraftMode::Team, &c).unwrap_err().contains("key"));
        assert!(validate_draft(&resp(vec![agent("a")], vec![pos("a", Some("zz"))]), DraftMode::Team, &c).unwrap_err().contains("supervisorKey"));
        let cyc = resp(vec![agent("a"), agent("b")], vec![pos("a", Some("b")), pos("b", Some("a"))]);
        assert!(validate_draft(&cyc, DraftMode::Team, &c).unwrap_err().contains("cycle"));
    }
    #[test] fn team_mode_requires_a_position_per_agent_and_size_cap() {
        let c = cat();
        assert!(validate_draft(&resp(vec![agent("a")], vec![]), DraftMode::Team, &c).unwrap_err().contains("positions"));
        let many: Vec<_> = (0..13).map(|i| agent(&format!("k{i}"))).collect();
        let ps: Vec<_> = (0..13).map(|i| pos(&format!("k{i}"), None)).collect();
        assert!(validate_draft(&resp(many, ps), DraftMode::Team, &c).unwrap_err().contains("12"));
    }
    #[test] fn rejects_unknown_role_skill_model_color_level() {
        let c = cat();
        let mut a = agent("a"); a.role_id = Some("nope".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("roleId"));
        let mut a = agent("a"); a.skill_ids = vec!["nope".into()];
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("skillIds"));
        let mut a = agent("a"); a.model = Some("gpt-5.5".into()); // codex model on claude-code
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("model"));
        let mut a = agent("a"); a.color = Some("#123456".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("color"));
        let mut a = agent("a"); a.default_level = Some("boss".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("defaultLevel"));
    }
    #[test] fn role_id_xor_new_role_and_new_role_name_must_be_fresh() {
        let c = cat();
        let mut a = agent("a"); a.new_role = Some(DraftNewRole { name: "QA".into(), description: "d".into(), skill_ids: vec!["agent-loop".into()] });
        assert!(validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c).unwrap_err().contains("roleId"));
        a.role_id = None;
        assert!(validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c).is_ok());
        a.new_role.as_mut().unwrap().name = "lead".into();
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("newRole.name"));
    }
    #[test] fn existing_def_reuse_must_exist_and_carry_no_other_fields() {
        let c = cat();
        let a = DraftAgent { key: "x".into(), existing_agent_def_id: Some("def-existing".into()), name: None, color: None, cli_kind: None, model: None, role_id: None, new_role: None, skill_ids: vec![], default_level: None, rationale: "r".into() };
        assert!(validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c).is_ok());
        let mut bad = a.clone(); bad.existing_agent_def_id = Some("ghost".into());
        assert!(validate_draft(&resp(vec![bad], vec![]), DraftMode::Agent, &c).unwrap_err().contains("existingAgentDefId"));
        let mut bad = a; bad.name = Some("N".into());
        assert!(validate_draft(&resp(vec![bad], vec![]), DraftMode::Agent, &c).unwrap_err().contains("existingAgentDefId"));
    }
    #[test] fn new_agent_needs_name_cli_kind_model_and_a_role() {
        let c = cat();
        let mut a = agent("a"); a.name = None;
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("name"));
        let mut a = agent("a"); a.role_id = None;
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).unwrap_err().contains("role"));
    }
    #[test] fn schema_lists_every_field_and_requires_key_and_rationale() {
        let s = draft_schema(DraftMode::Team);
        let props = &s["properties"]["agents"]["items"]["properties"];
        for f in ["key","existingAgentDefId","name","color","cliKind","model","roleId","newRole","skillIds","defaultLevel","rationale"] {
            assert!(props.get(f).is_some(), "missing {f}");
        }
        assert_eq!(s["properties"]["agents"]["items"]["required"], serde_json::json!(["key","rationale"]));
        assert!(s["properties"].get("positions").is_some());
        assert_eq!(s["required"], serde_json::json!(["agents","positions","notes"]));
    }
}
```

`SkillRow` must be `Deserialize` for the `skill()` helper; if it is not, construct it by struct literal (fields at `repo/skill.rs:20-34`).

- [ ] **Step 2: Run to verify failure** — `cd src-tauri && cargo test commands::draft` → compile error.

- [ ] **Step 3: Implement** the types above plus:

```rust
pub fn draft_schema(mode: DraftMode) -> Value {
    let agent = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["key", "rationale"],
        "properties": {
            "key": {"type": "string", "description": "Draft-local handle, unique, e.g. \"lead\", \"impl-1\"."},
            "existingAgentDefId": {"type": "string", "description": "Reuse this existing agent definition id. When set, give NO other field besides key and rationale."},
            "name": {"type": "string", "maxLength": 40},
            "color": {"type": "string", "enum": COLOR_SWATCHES},
            "cliKind": {"type": "string", "enum": ["claude-code", "codex"]},
            "model": {"type": "string", "description": "A model id from the catalogue for the chosen cliKind."},
            "roleId": {"type": "string", "description": "An existing role id from the catalogue. Mutually exclusive with newRole."},
            "newRole": {"type": "object", "additionalProperties": false, "required": ["name", "description", "skillIds"],
                        "properties": {"name": {"type": "string", "maxLength": 40}, "description": {"type": "string", "maxLength": 600},
                                       "skillIds": {"type": "array", "items": {"type": "string"}}}},
            "skillIds": {"type": "array", "items": {"type": "string"}, "description": "Optional skill ids from the catalogue (mandatory skills are attached automatically)."},
            "defaultLevel": {"type": "string", "enum": LEVELS},
            "rationale": {"type": "string", "maxLength": 200}
        }
    });
    let max_agents = match mode { DraftMode::Agent => 1, DraftMode::Team => MAX_TEAM_SIZE };
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["agents", "positions", "notes"],
        "properties": {
            "agents": {"type": "array", "minItems": 1, "maxItems": max_agents, "items": agent},
            "positions": {"type": "array", "items": {"type": "object", "additionalProperties": false, "required": ["key", "level"],
                          "properties": {"key": {"type": "string"}, "level": {"type": "string", "enum": LEVELS},
                                         "supervisorKey": {"type": ["string", "null"]}}}},
            "notes": {"type": "string", "maxLength": 600, "description": "One short paragraph for the user: assumptions and anything the brief left open."}
        }
    })
}
```

Validator (`validate_draft`), in this order, each returning `Err(format!("draft.{field}: {reason}"))`:
1. `agents` non-empty; Agent mode → `agents.len() == 1` and `positions.is_empty()` (fields `draft.agents` / `draft.positions`); Team mode → `agents.len() <= MAX_TEAM_SIZE` (reason contains "12").
2. Keys unique (`draft.agents[i].key: duplicate`).
3. Per agent: if `existing_agent_def_id` is Some → must be in `cat.existing` ids and every other optional field None / `skill_ids` empty (`draft.agents[i].existingAgentDefId`). Else: `name` Some+non-empty (`.name`), `cli_kind` ∈ {claude-code, codex} (`.cliKind`), `model` ∈ the list for that cli_kind (`.model`), exactly one of `role_id`/`new_role` (`.roleId: give roleId or newRole, not both/neither` — the test expects the string "roleId" for the both case and "role" for neither), `role_id` ∈ `cat.roles` ids (`.roleId`), `new_role.name` not equal (case-insensitive) to any `cat.roles[].name` or id (`.newRole.name`), `new_role.skill_ids` and `skill_ids` ⊆ `cat.skills` ids (`.skillIds` / `.newRole.skillIds`), `color` ∈ COLOR_SWATCHES if Some (`.color`), `default_level` ∈ LEVELS if Some (`.defaultLevel`).
4. Team mode: every agent key has exactly one position and vice versa (`draft.positions: every agent needs one position`); `level` ∈ LEVELS; `supervisor_key` names another agent key (`draft.positions[i].supervisorKey`); no cycle — walk supervisor links from each key with a visited set, error text contains "cycle".

`build_catalogue`: roles = `repo::role::list_builtin()` followed by `repo::role::list(db).await?`; skills = `repo::skill::list_builtin()` + `repo::skill::list(db).await?` filtered to `!mandatory`; existing = `repo::agent_definition::list(db).await?` mapped to `ExistingDef` (role_name = `role` column fallback, or resolve `role_id` through the roles vec); roster = when `workspace_id` is Some, `repo::workspace_agent::list_by_workspace(db, ws)` joined with the defs for names, supervisor name resolved within the same list.

Register `pub mod draft;` in `commands/mod.rs`.

- [ ] **Step 4: Run the gate** — `cd src-tauri && cargo test commands::draft && cargo clippy --all-targets -- -D warnings && rustfmt --check src/engine/commands/draft.rs src/engine/commands/mod.rs` → 8 passed.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(draft): wire types, JSON schema, catalogue and validator for AI agent/team drafts" -- src-tauri/src/engine/commands/draft.rs src-tauri/src/engine/commands/mod.rs
```

### Task A4: Prompt builder, `draft.agents` handler, router

**Files:**
- Create: `src-tauri/src/engine/commands/draft_prompt.rs`
- Modify: `src-tauri/src/engine/commands/draft.rs` (add `run`, `run_with`), `commands/mod.rs` (`pub mod draft_prompt;`), `router.rs` (route)

**Interfaces:**
- Produces: `pub fn build_prompt(mode: DraftMode, brief: &str, cat: &Catalogue, schema: &Value) -> String`; `pub async fn run(state: &AppState, payload: Value) -> Result<Value, AppError>`; `pub async fn run_with(db: &SqlitePool, oneshot: &Oneshot, req: DraftRequest) -> Result<DraftResponse, AppError>`.
- Router: `"draft.agents" => draft::run(state, payload).await,` placed under a new `// ── draft ──` comment block.

- [ ] **Step 1: Write the failing tests** (in `draft_prompt.rs` and `draft.rs`)

```rust
// draft_prompt.rs tests
#[test]
fn prompt_embeds_catalogue_ids_brief_and_schema() {
    let c = /* same cat() helper as draft.rs tests — re-declare it here or make it pub(crate) in draft::tests */;
    let schema = draft_schema(DraftMode::Team);
    let p = build_prompt(DraftMode::Team, "Port the billing service to Rust", &c, &schema);
    for needle in ["implementer", "agent-loop", "def-existing", "claude-sonnet-5", "gpt-5.5", "Port the billing service to Rust", "\"agents\""] {
        assert!(p.contains(needle), "missing {needle}");
    }
    assert!(p.contains("exactly one top-level lead"));
    assert!(!build_prompt(DraftMode::Agent, "x", &c, &schema).contains("exactly one top-level lead"));
}

// draft.rs tests (append)
#[tokio::test]
async fn run_with_mock_returns_validated_response() {
    let db = crate::engine::db::test_pool().await; // use the crate's existing in-memory test-pool helper; grep `async fn test_pool` or how fusion.rs tests open a db
    let def = /* insert a cli claude-code AgentDefinition via repo::agent_definition::create — copy the call shape from agent.rs tests */;
    let canned = serde_json::json!({
        "agents": [{"key":"lead","name":"Nova","color": COLOR_SWATCHES[0],"cliKind":"claude-code","model":"claude-sonnet-5","roleId":"lead","skillIds":[],"defaultLevel":"principal","rationale":"r"}],
        "positions": [{"key":"lead","level":"principal","supervisorKey":null}],
        "notes": "n"
    });
    let req = DraftRequest { mode: DraftMode::Team, brief: "b".into(), drafter_def_id: def.id.clone(), workspace_id: None };
    let out = run_with(&db, &Oneshot::Mock(Ok(canned)), req).await.unwrap();
    assert_eq!(out.agents[0].name.as_deref(), Some("Nova"));
    assert_eq!(out.drafter.def_id, def.id);
    assert_eq!(out.drafter.cli_kind, "claude-code");
}

#[tokio::test]
async fn run_with_rejects_invalid_model_output_as_invalid() {
    /* same setup; canned agents[0].roleId = "ghost" → expect Err(AppError::Invalid(msg)) with msg containing "roleId" */
}

#[tokio::test]
async fn run_with_rejects_empty_brief_and_non_cli_drafter() {
    /* brief "   " → Invalid("draft.brief …"); drafter def with type "chat" → Invalid("draft.drafterDefId …") — checked BEFORE the oneshot runs (Mock(Err) must not be reached: use Mock(Err("must not run")) and assert the error is the Invalid, not Model) */
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test draft` → compile errors.

- [ ] **Step 3: Implement `draft_prompt.rs`**

```rust
//! Prompt text for `commands::draft`. Const fragments + one builder so tests
//! can assert the catalogue is embedded. English only.
use serde_json::Value;
use super::draft::{Catalogue, DraftMode, CLAUDE_MODELS, CODEX_MODELS, COLOR_SWATCHES, LEVELS};

const TASK_AGENT: &str = "You are configuring ONE AI agent definition for Conclave, a macOS app that runs Claude Code / Codex agents as a team inside a project workspace. Draft the single best-fitting agent for the brief.";
const TASK_TEAM: &str = "You are staffing a TEAM of AI agents for Conclave, a macOS app that runs Claude Code / Codex agents as a team inside a project workspace. Draft the smallest team that covers the brief, with reporting lines.";
const RULES_COMMON: &str = "Rules:\n- Use ONLY ids that appear in the catalogue below (roles, skills, models, colours, levels). Never invent an id.\n- Prefer an existing agent definition (existingAgentDefId) when one already fits the job; then give no other fields for that entry.\n- Propose newRole only when no catalogue role fits; give it a concrete one-paragraph description written as standing instructions to the agent.\n- Names are short, distinctive, human-like (one word), unique within the draft and not already used in the catalogue.\n- rationale: one sentence. notes: one short paragraph of assumptions. Output English only.";
const RULES_TEAM: &str = "- Team shape: exactly one top-level lead (no supervisor) at level principal; every other agent has a supervisorKey; reviewers and researchers never supervise implementers; keep it to the fewest agents that cover the brief (max 12).\n- If a current roster is listed, EXTEND it: reuse those members via existingAgentDefId where sensible and do not duplicate their jobs.";
const LEVEL_MEANING: &str = "Levels: junior (executes well-specified tasks), mid (owns a task end to end), senior (owns a lane, reviews peers), principal (leads, rules on disputes).";

pub fn build_prompt(mode: DraftMode, brief: &str, cat: &Catalogue, schema: &Value) -> String {
    let mut p = String::new();
    p.push_str(match mode { DraftMode::Agent => TASK_AGENT, DraftMode::Team => TASK_TEAM });
    p.push_str("\n\nReply with ONE JSON object matching this schema exactly:\n");
    p.push_str(&schema.to_string());
    p.push_str("\n\n");
    p.push_str(RULES_COMMON);
    if mode == DraftMode::Team { p.push('\n'); p.push_str(RULES_TEAM); }
    p.push_str("\n\n## Catalogue\n\n### Roles (id — name: description; default skills)\n");
    for r in &cat.roles {
        p.push_str(&format!("- {} — {}: {}; skills: {}\n", r.id, r.name, r.description.trim(), r.skill_ids.join(", ")));
    }
    p.push_str("\n### Optional skills (id — name: description)\n");
    for s in &cat.skills {
        p.push_str(&format!("- {} — {}: {}\n", s.id, s.name, s.description.as_deref().unwrap_or("").trim()));
    }
    p.push_str(&format!("\n### Models\n- claude-code: {}\n- codex: {}\n", CLAUDE_MODELS.join(", "), CODEX_MODELS.join(", ")));
    p.push_str(&format!("\n### Colours\n{}\n", COLOR_SWATCHES.join(", ")));
    p.push_str(&format!("\n### Levels\n{}\n{}\n", LEVELS.join(", "), LEVEL_MEANING));
    p.push_str("\n### Existing agent definitions (id — name, role, cliKind/model)\n");
    if cat.existing.is_empty() { p.push_str("(none)\n"); }
    for d in &cat.existing {
        p.push_str(&format!("- {} — {}, {}, {}/{}\n", d.id, d.name, d.role_name.as_deref().unwrap_or("no role"), d.cli_kind.as_deref().unwrap_or("-"), d.model.as_deref().unwrap_or("-")));
    }
    if mode == DraftMode::Team {
        p.push_str("\n### Current roster of this workspace (name — role, level, reports to)\n");
        if cat.roster.is_empty() { p.push_str("(empty)\n"); }
        for m in &cat.roster {
            p.push_str(&format!("- {} — {}, {}, reports to {}\n", m.name, m.role_name.as_deref().unwrap_or("no role"), m.level.as_deref().unwrap_or("-"), m.supervisor_name.as_deref().unwrap_or("nobody")));
        }
    }
    p.push_str("\n## Brief\n\n```\n");
    p.push_str(brief.trim());
    p.push_str("\n```\n");
    p
}
```

- [ ] **Step 4: Implement `run` / `run_with` in `draft.rs`**

```rust
pub async fn run(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DraftRequest = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let out = run_with(&state.db, &Oneshot::Live, req).await?;
    serde_json::to_value(out).map_err(|e| AppError::Internal(e.to_string()))
}

pub async fn run_with(db: &SqlitePool, oneshot: &Oneshot, req: DraftRequest) -> Result<DraftResponse, AppError> {
    let brief = req.brief.trim();
    if brief.is_empty() { return Err(AppError::Invalid("draft.brief: brief is empty".into())); }
    if brief.chars().count() > BRIEF_MAX_CHARS { return Err(AppError::Invalid(format!("draft.brief: longer than {BRIEF_MAX_CHARS} characters"))); }
    let def = repo::agent_definition::get(db, &req.drafter_def_id).await?
        .ok_or_else(|| AppError::Invalid("draft.drafterDefId: no such agent definition".into()))?;
    let cli_kind = def.cli_kind.as_deref().and_then(CliKind::parse)
        .filter(|_| def.r#type == "cli")
        .ok_or_else(|| AppError::Invalid("draft.drafterDefId: the drafter must be a Claude Code or Codex CLI agent".into()))?;
    let model = def.model.clone().filter(|m| !m.is_empty());
    let model_for_launch = match cli_kind {
        CliKind::ClaudeCode => model.as_deref().map(|m| effective_claude_model(m, def.context_window.as_deref())),
        CliKind::Codex => model.clone(),
    };
    let cat = build_catalogue(db, req.workspace_id.as_deref()).await?;
    let schema = draft_schema(req.mode);
    let prompt = build_prompt(req.mode, brief, &cat, &schema);
    let cwd = match req.workspace_id.as_deref() {
        Some(ws) => repo::workspace::get(db, ws).await?.map(|w| PathBuf::from(w.folder_path)).unwrap_or_else(std::env::temp_dir),
        None => std::env::temp_dir(),
    };
    let spec = OneshotSpec { cli_kind, model: model_for_launch, prompt, json_schema: schema, extra_env: agent_env_overrides(&def), cwd, timeout: Duration::from_secs(120) };
    let raw = oneshot.run(&spec).await.map_err(|e| AppError::Invalid(format!("draft.run: {e}")))?;
    #[derive(serde::Deserialize)] #[serde(rename_all = "camelCase")]
    struct ModelOut { agents: Vec<DraftAgent>, #[serde(default)] positions: Vec<DraftPosition>, #[serde(default)] notes: String }
    let parsed: ModelOut = serde_json::from_value(raw).map_err(|e| AppError::Invalid(format!("draft.parse: {e}")))?;
    let resp = DraftResponse { agents: parsed.agents, positions: parsed.positions, notes: parsed.notes,
        drafter: DrafterInfo { def_id: def.id.clone(), cli_kind: def.cli_kind.clone().unwrap_or_default(), model: model.unwrap_or_default() } };
    validate_draft(&resp, req.mode, &cat).map_err(AppError::Invalid)?;
    Ok(resp)
}
```

Add the router arm and `pub mod draft_prompt;`. Timeout/exit/model errors surface as `AppError::Invalid("draft.run: …")` so the frontend shows the message verbatim (spec error table).

- [ ] **Step 5: Run the gate** — `cd src-tauri && cargo test draft && cargo clippy --all-targets -- -D warnings && rustfmt --check src/engine/commands/draft.rs src/engine/commands/draft_prompt.rs src/engine/commands/mod.rs src/engine/router.rs`.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(draft): draft.agents command — prompt builder, one-shot run, validated response" -- src-tauri/src/engine/commands/draft.rs src-tauri/src/engine/commands/draft_prompt.rs src-tauri/src/engine/commands/mod.rs src-tauri/src/engine/router.rs
```

### Task A5: Manual gate — a real `claude -p` run of the shipped prompt (and codex if logged in)

**Files:** none changed (evidence only). Optional: `scripts/draft-probe.sh` if you need a repeatable wrapper — keep it under 30 lines.

- [ ] **Step 1: Build a probe** that prints the exact prompt and schema: add a `#[test] #[ignore] fn dump_prompt_for_probe()` in `draft_prompt.rs` that writes `build_prompt(Team, "Port the billing service from Node to Rust with tests and a reviewer", &cat(), &schema)` to `$CONCLAVE_PROBE_OUT/prompt.txt` and `schema.json` (skip when the env var is unset).
- [ ] **Step 2: Run the shipped launch string by hand**

```bash
export CONCLAVE_PROBE_OUT=/tmp/draft-probe && mkdir -p $CONCLAVE_PROBE_OUT
cd src-tauri && cargo test dump_prompt_for_probe -- --ignored
cd /tmp/draft-probe && claude -p --output-format json --json-schema "$(cat schema.json)" --no-session-persistence --tools '' --model claude-sonnet-5 < prompt.txt > claude.out
python3 -c "import json;d=json.load(open('claude.out'));print(d['subtype'],json.dumps(d['structured_output'],indent=1)[:1500])"
```
Expected: `success` and a team with one principal lead, supervisors set, ids only from the catalogue. If `subtype != success`, fix the prompt, not the validator.
- [ ] **Step 3: Codex** — if `codex login status` is OK, run `codex exec --json --ephemeral --skip-git-repo-check --output-schema schema.json -o last.json -m gpt-5.5 - < prompt.txt` and check `last.json` parses. If codex is not logged in, record the skip.
- [ ] **Step 4: Record** — `conclave task gate <ws> drafter-oneshot-rust -- cat /tmp/draft-probe/claude.out` and a task note with the observed `structured_output` summary (agents count, lead key, any validator complaint). Then state → review.

---

# Lane B — Design canon (Arta)

### Task B1: Canon for the AgentDrafter overlay and entry buttons

**Deliverable:** a proto under the designer's canon folder (the path convention Arta already uses for canons, e.g. `design/…` or `.arta/proto/screens/` — use whichever the last UI canon task `agent-stop-ui-canon-v2` used) plus a PNG, with the SHA pinned in the task note.

Screens/states to draw, in Conclave's existing Builder/Library visual language (PRODUCT.md: quiet, dense, native):
1. **Entry points** — a secondary button "Draft with AI" beside Library's "New agent" (`Library.tsx:235-244`), and "Build team with AI" beside Roster's "Add agent" (`Roster.tsx:701-710`). Sparkles icon (lucide `Sparkles`, already imported in Builder).
2. **Drafter overlay, empty state** — title "Draft an agent" / "Build a team", brief textarea (placeholder: "Describe the job. Example: a team to port our billing service from Node to Rust with tests and a reviewer."), drafter picker (label "Drafter", lists CLI definitions, shows cliKind/model), primary button "Draft". No-CLI-drafter variant: inline notice "Configure a Claude Code or Codex agent first" + "Open Builder".
3. **Waiting** — drafter name + elapsed seconds, brief still visible, Draft disabled.
4. **Error** — message line + "Retry".
5. **Team preview** — editable table: Name, Role (select incl. "New: <name>"), Model, Level, Reports to, Reuse badge, Rationale (muted). Notes paragraph above the table. Footer: "Back", "Apply N agents".
6. **Apply progress** — per-row status (pending / created / added / positioned / failed) and a completion line "3 agents created and added to <workspace>".
7. **Builder with a draft** — a small "Drafted by <name>" chip under the Identity header that disappears on first edit; footer button reads "Create agent".

Gate: PNG opened and inspected; note the file path + SHA. Escalation target for design questions: Detoro (task owner).

---

# Lane C — Frontend (after A3's contract is committed and B1's canon is pinned)

### Task C1: Lift the model/colour catalogue and fix Builder's draft semantics

**Files:**
- Create: `src/lib/modelCatalogue.ts`
- Modify: `src/components/Builder.tsx:39-47` (swatches), `:74-90` (models), `:184` (`isEditing`), props at `:26-33`, footer `:1436-1450`

**Interfaces:**
- Produces: `export const CLAUDE_MODELS: string[]`, `export const CODEX_MODELS: string[]`, `export const COLOR_SWATCHES: string[]` (values byte-identical to Builder's current arrays); `BuilderProps.draftedBy?: string`.

- [ ] **Step 1:** Create `src/lib/modelCatalogue.ts` with the three exported arrays copied from Builder, then set `CLAUDE_MODELS = ["claude-fable-5-1", "claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5", "claude-opus-4-8"]` (human request 2026-09-04: the Claude 5 family from Claude Code's model picker — Fable 5.1, Opus 5, Sonnet 5, Haiku 4.5 — must be byte-identical to Rust `draft::CLAUDE_MODELS` in Task A3). In Builder delete the local consts and `import { CLAUDE_MODELS, CODEX_MODELS, COLOR_SWATCHES } from "../lib/modelCatalogue";` — the Builder's model preset list gains the new ids for free.
- [ ] **Step 2:** Change `const isEditing = Boolean(initialDef);` to `const isEditing = Boolean(initialDef?.id);` — an id-less `initialDef` is a draft, not an edit (spec R6). Grep `isEditing` in Builder to confirm every use (copy, role-transition logic at :307-360) is correct for the draft case.
- [ ] **Step 3:** Add `draftedBy?: string` to `BuilderProps`; keep `const [touched, setTouched] = useState(false)`; render under the Identity header when `draftedBy && !touched`: a chip `<span className="text-[11px] text-text-tertiary inline-flex items-center gap-1"><Sparkles className="w-3 h-3" />Drafted by {draftedBy}</span>`; set `touched` in the `onChange` of the name input and the role card click (two places is enough).
- [ ] **Step 4:** Gate — `pnpm exec tsc --noEmit && pnpm build && pnpm uishot builder` → open `.shots/builder-default.png`, confirm the Builder renders unchanged.
- [ ] **Step 5:** Commit — `git commit -m "refactor(builder): lift model/colour catalogue; id-less initialDef is a draft, with a drafted-by chip" -- src/lib/modelCatalogue.ts src/components/Builder.tsx`.

### Task C2: IPC contract, types, fixtures

**Files:**
- Modify: `src/ipc/types.ts` (append), `src/ipc/commands.ts` (Commands map + `ipc.draft`), `src/ipc/index.ts` (re-export), `src/fixtures/scenarios/data.ts` (add `draftTeam`), `default.ts`, `empty.ts`

**Interfaces:**
- Produces (mirror of Task A3, verbatim):

```ts
// src/ipc/types.ts
export type DraftMode = "agent" | "team";
export type DraftLevel = "junior" | "mid" | "senior" | "principal";
export interface DraftNewRole { name: string; description: string; skillIds: string[] }
export interface DraftAgent {
  key: string;
  existingAgentDefId?: string;
  name?: string;
  color?: string;
  cliKind?: "claude-code" | "codex";
  model?: string;
  roleId?: string;
  newRole?: DraftNewRole;
  skillIds: string[];
  defaultLevel?: DraftLevel;
  rationale: string;
}
export interface DraftPosition { key: string; level: DraftLevel; supervisorKey?: string | null }
export interface DrafterInfo { defId: string; cliKind: string; model: string }
export interface DraftResponse { agents: DraftAgent[]; positions: DraftPosition[]; notes: string; drafter: DrafterInfo }
```

```ts
// src/ipc/commands.ts — inside Commands
"draft.agents": {
  req: { mode: DraftMode; brief: string; drafterDefId: string; workspaceId?: string };
  res: DraftResponse;
};
// ergonomic wrapper, beside `role:`
draft: { agents: (req: Commands["draft.agents"]["req"]) => call("draft.agents", req) },
```

- [ ] **Step 1:** Add the types, command entry, wrapper and re-exports (`src/ipc/index.ts` re-export list at :4-45).
- [ ] **Step 2:** Fixture data in `data.ts`:

```ts
export const draftTeam: DraftResponse = {
  drafter: { defId: agentDefs[0].id, cliKind: "claude-code", model: "claude-sonnet-5" },
  notes: "Assumed the port keeps the public HTTP contract; added a reviewer because the brief asks for tests.",
  agents: [
    { key: "lead", name: "Nova", color: "#5e5ce6", cliKind: "claude-code", model: "claude-opus-4-8", roleId: "lead", skillIds: ["agent-loop"], defaultLevel: "principal", rationale: "One lead settles decisions and integrates." },
    { key: "impl-rust", name: "Ferro", color: "#ff9f0a", cliKind: "claude-code", model: "claude-sonnet-5", newRole: { name: "Rust Porter", description: "You port Node services to idiomatic Rust, module by module, keeping behaviour identical and covering each module with tests before moving on.", skillIds: ["implementer"] }, skillIds: ["implementer"], defaultLevel: "senior", rationale: "The brief needs a Rust specialist; no catalogue role fits." },
    { key: "reviewer", existingAgentDefId: agentDefs[1].id, skillIds: [], rationale: "Reuse the existing reviewer definition." },
  ],
  positions: [
    { key: "lead", level: "principal", supervisorKey: null },
    { key: "impl-rust", level: "senior", supervisorKey: "lead" },
    { key: "reviewer", level: "senior", supervisorKey: "lead" },
  ],
};
```
Use ids that exist in `agentDefs` / `roles` / `skills` of `data.ts` (check `roles` at ~:615 and `skills` at ~:647); if `agentDefs[1]` is not a reviewer, pick the one that is.
- [ ] **Step 3:** Handlers: `default.ts` → `"draft.agents": ({ mode }) => mode === "agent" ? { ...draftTeam, agents: [draftTeam.agents[1]], positions: [] } : draftTeam,`; `empty.ts` → `"draft.agents": () => ({ agents: [], positions: [], notes: "", drafter: { defId: "", cliKind: "", model: "" } }),`.
- [ ] **Step 4:** Gate — `pnpm exec tsc --noEmit`. Commit — `git commit -m "feat(ipc): draft.agents contract, types and fixtures" -- src/ipc/types.ts src/ipc/commands.ts src/ipc/index.ts src/fixtures/scenarios/data.ts src/fixtures/scenarios/default.ts src/fixtures/scenarios/empty.ts`.

### Task C3: `src/lib/applyTeamDraft.ts` — topological order and the apply executor

**Files:**
- Create: `src/lib/applyTeamDraft.ts`

**Interfaces:**
- Consumes: `ipc.role.save`, `ipc.agentDef.save` (request shape: copy the object literal from `Builder.tsx` `handleSave` ~:453 and fill: `id: undefined, name, color, type: "cli", cliKind, model, roleId, skillIds, defaultLevel, harnessMode: "own"`, everything else omitted), `ipc.agentDef.addToWorkspace({ agentDefId, workspaceIds: [workspaceId] })`, `ipc.instance.list({ workspaceId })`, `ipc.instance.setPosition`.
- Produces:

```ts
export type ApplyStatus = "pending" | "created" | "added" | "positioned" | "failed" | "skipped";
export interface ApplyProgress { key: string; status: ApplyStatus; message?: string }
export interface ApplyResult { created: number; failedKey?: string; error?: string }
/** Keys ordered so every supervisor precedes its reports. Throws on a cycle. */
export function topoOrder(positions: DraftPosition[]): string[];
export async function applyTeamDraft(
  draft: DraftResponse, workspaceId: string, onProgress: (p: ApplyProgress) => void,
): Promise<ApplyResult>;
```

- [ ] **Step 1:** Implement `topoOrder` (Kahn's algorithm over `supervisorKey` edges; unknown supervisor → treat as root; cycle → `throw new Error("cycle in reporting lines")`).
- [ ] **Step 2:** Implement `applyTeamDraft`:
  1. `const roster = await ipc.instance.list({ workspaceId })`; `const keyToWsAgent = new Map<string, string>()`.
  2. For each key in `topoOrder(draft.positions)`: find the agent; `onProgress({key, status:"pending"})`.
     - `defId`: if `existingAgentDefId` → it; else if `newRole` → `const role = await ipc.role.save({ name, description, skillIds })` then `roleId = role.id`; then `const def = await ipc.agentDef.save({...})` → `def.id`; `onProgress created`.
     - If the roster already has a row with `agentDefId === defId` → reuse its `id` (status `skipped` for the add step); else `await ipc.agentDef.addToWorkspace({ agentDefId: defId, workspaceIds: [workspaceId] })`, then `const after = await ipc.instance.list({ workspaceId })` and take the row with `agentDefId === defId` that was not in `roster`; `onProgress added`.
     - Position: `supervisorAgentId = position.supervisorKey ? keyToWsAgent.get(position.supervisorKey) ?? null : null`; `await ipc.instance.setPosition({ workspaceId, workspaceAgentId, level: position.level, supervisorAgentId })`; `onProgress positioned`; `keyToWsAgent.set(key, workspaceAgentId)`.
     - Any throw → `onProgress({key, status:"failed", message: String(err)})` and `return { created, failedKey: key, error: String(err) }`.
  3. Return `{ created }`.
- [ ] **Step 3:** Gate — `pnpm exec tsc --noEmit`. Commit — `git commit -m "feat(draft): applyTeamDraft — topological apply over existing role/agentDef/position commands" -- src/lib/applyTeamDraft.ts`.

### Task C4: `AgentDrafter.tsx` overlay + AppShell/Library/Roster wiring

**Files:**
- Create: `src/components/AgentDrafter.tsx`
- Modify: `src/components/AppShell.tsx` (state, view map, render, Builder key/props), `Library.tsx:8-14, 235-244`, `Roster.tsx:286-294, 701-710`

**Interfaces:**
- `AgentDrafterProps { mode: DraftMode; workspaceId?: string; workspaceName?: string; onClose: () => void; onDraftAgent: (def: AgentDefinition, draftedBy: string) => void; onTeamApplied: () => void; onOpenBuilder: () => void }`
- `export function draftToInitialDef(a: DraftAgent, roleName?: string): AgentDefinition` — id-less object: `{ id: "", name, color, type: "cli", cliKind, model, roleId, skillIds, defaultLevel, harnessMode: "own", createdAt: "" }` (when `newRole` is present the Builder cannot show it, so agent mode first calls `ipc.role.save` for the new role and passes its id; when `existingAgentDefId` is present, pass that def fetched via `agentDef.list`).
- AppShell: `const [showDrafter, setShowDrafter] = useState<{ mode: DraftMode } | null>(null); const [draftSeq, setDraftSeq] = useState(0); const [builderDraftedBy, setBuilderDraftedBy] = useState<string | undefined>()`; view map adds `drafter: () => setShowDrafter({ mode: "team" })`; Builder `key={builderInitialDef?.id || \`draft-${draftSeq}\`}` and `draftedBy={builderDraftedBy}`; `onDraftAgent={(def, by) => { setBuilderInitialDef(def); setBuilderDraftedBy(by); setDraftSeq(s => s + 1); setShowDrafter(null); setShowBuilder(true); }}`; `onTeamApplied={() => { setShowDrafter(null); setLibraryRefreshKey(k => k + 1); setAgentsVersion(v => v + 1); }}`.
- Library: new prop `onOpenDrafter: () => void`, button beside "New agent". Roster: new prop `onBuildTeam?: () => void`, button beside "Add agent" (disabled when `workspaceId === null`).

- [ ] **Step 1:** Build the component with these states: `phase: "idle" | "running" | "error" | "preview" | "applying" | "done"`. On mount load `ipc.agentDef.list()` filtered like `SkillAssistPanel.tsx:67-77`. Elapsed timer via `setInterval` while running. Draft button → `ipc.draft.agents({ mode, brief, drafterDefId, workspaceId })`. In DEV fixture mode (`fixtureScenario()` from `src/fixtures/mode.ts` truthy) auto-run once on mount with a sample brief so `pnpm uishot drafter` renders the preview. Preview table per B1's canon; edits mutate a local copy of `DraftResponse` (`name`, `roleId`/`newRole` select, `model` select from the catalogue per cliKind, `level`, `supervisorKey` select). Apply → `applyTeamDraft` with progress rows; on `{failedKey}` show the error and the created count; on success show the completion line and a "Done" button → `onTeamApplied()`. Agent mode: on success call `onDraftAgent(draftToInitialDef(...), drafterName)` directly (no preview).
- [ ] **Step 2:** Wire AppShell/Library/Roster as above. Overlay markup: copy the Builder's outer shell (backdrop + centred panel + header + scroll body + footer, `Builder.tsx:534-560` and `:1436-1450`) so the two overlays look identical.
- [ ] **Step 3:** Gate — `lsof -nP -iTCP:1420 -sTCP:LISTEN` (kill foreign vite servers), then `pnpm exec tsc --noEmit && pnpm build && pnpm uishot drafter && pnpm uishot drafter --scenario empty && pnpm uishot library && pnpm uishot home`. Open every PNG and check: preview table with 3 rows incl. a "New: Rust Porter" role and a Reuse badge (default), the no-drafter empty state (empty), the two new buttons. Record: `conclave task gate <ws> drafter-frontend -- pnpm uishot drafter`.
- [ ] **Step 4:** Commit — `git commit -m "feat(ui): AgentDrafter overlay — draft an agent or build a team with AI, apply via existing commands" -- src/components/AgentDrafter.tsx src/components/AppShell.tsx src/components/Library.tsx src/components/Roster.tsx`.

### Task C5: Manual end-to-end against the real engine

- [ ] Run the app (`pnpm tauri dev`), open Roster → "Build team with AI", brief "Port the billing service from Node to Rust with tests and a reviewer", drafter = a Claude Code definition. Expect a preview within ~60 s; Apply; confirm Library shows the new definitions, Roster shows them with level/supervisor chips. Then Library → "Draft with AI" → Builder opens pre-filled with the "Drafted by" chip. Attach a note with what you saw (agents, timing, any validator error text) on the task and move it to review.

---

## Risk ledger (from the spec, with the task that owns each)

- R1 shell banners on stdout → A2 `extract_last_json_object`; if the envelope is still missing in A5, drop `-i` from the one-shot args in A2 and re-gate.
- R2 codex `--output-schema` unverified → A5 Step 3; fallback is already coded (`parse_codex_last_message` strips fences).
- R4 `instance.rs` hoist → A1 is a pure move landed first, before any other lane touches the file.
- R6 Builder `isEditing` → C1 Step 2.
- `agentDef.addToWorkspace` does not return the new workspace-agent id → C3 re-lists the roster and diffs.
- `AgentDefRow` / `SkillRow` may not derive `Deserialize` → A1/A3 test helpers fall back to struct literals.
