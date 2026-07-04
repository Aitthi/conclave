# Harness sandbox ↔ conclave CLI — zero-prompt config (research)

**Date:** 2026-07-04 · **Researcher:** Guetta (2b110fd3) · **Owner/escalation:** Detoro (bfb737ff) · **Authority:** in-loop
**Claim:** `claim:harness-sandbox-conclave-cli` · **Blackboard:** `plan:` / `progress:harness-sandbox-conclave-cli`
**Scope:** research only, no code changes.

## The problem (human, 2026-07-04)

In auto permission mode, spawned agents can't use the conclave CLI cleanly:

- **claude-code:** a one-time macOS **sandbox (seatbelt) modal** prompts, then works.
- **codex:** exec fails entirely unless bypass mode.

Root cause (now **confirmed**): the CLI binary **and** its UDS socket both live at
`~/Library/Application Support/Conclave/` — **outside** the workspace:

- binary: `/Users/detoro/Library/Application Support/Conclave/bin/conclave`
- socket: `/Users/detoro/Library/Application Support/Conclave/conclave.sock` (mode `srw-------`, UDS)

Under each harness's OS sandbox the binary **executes fine**, but the `connect()` to the
out-of-workspace AF_UNIX socket is denied → `Operation not permitted (os error 1)` (EPERM).
The fix is a spawn-time config that pokes exactly one hole for that socket, keeping the sandbox
on for everything else. **Constraint honored:** zero extra user action, auto mode stays enabled,
no full bypass.

Environment tested on this machine: **claude-code 2.1.201**, **codex-cli 0.142.5**, macOS (darwin 25.5.0).

---

## codex — CONFIRMED empirically

`codex sandbox` runs a command under the same seatbelt/permissions stack the agent's shell
tool calls use. All results below are from running the real read-only command
`conclave bb get <ws> <key>` under it, cwd = the workspace `/Users/detoro/code/codeup`.

| # | Config | Result |
|---|--------|--------|
| A | baseline (workspace-write, no allowance) | **EPERM** on socket connect; binary *did* exec |
| B | `--allow-unix-socket <sock>` (CLI flag) | ✅ success |
| C | `--allow-unix-socket <parent dir>` | ✅ success ("rooted at path" covers the subtree) |
| J | `[permissions.conclave]` profile (below) | ✅ **success** |
| K | same profile **minus** the socket grant | ❌ EPERM (negative control — the socket key is what fixes it) |

Rejected along the way (all still EPERM): `sandbox_workspace_write.allow_unix_sockets`,
`experimental_network.unix_sockets` (+enabled), `features.network_proxy.*`,
`dangerously_allow_all_unix_sockets`. The `sandbox_workspace_write` policy struct has only
`writable_roots` / `exclude_tmpdir_env_var` / `exclude_slash_tmp` — **no** unix-socket field;
unix sockets live in the **permissions-profile / network** layer, not the fs-sandbox layer.

### Recommended codex injection (self-contained, proven in test J)

Pass these `-c` overrides at spawn (works on `codex exec` / `codex` the same way it did on
`codex sandbox`; `-c` = dotted path, value parsed as TOML):

```
codex \
  -c 'permissions.conclave.extends=":workspace"' \
  -c 'permissions.conclave.network.enabled=true' \
  -c 'permissions.conclave.network.unix_sockets={"/Users/detoro/Library/Application Support/Conclave/conclave.sock"="allow"}' \
  -c 'default_permissions="conclave"' \
  …
```

Equivalent persistent form in `~/.codex/config.toml`:

```toml
default_permissions = "conclave"

[permissions.conclave]
extends = ":workspace"
network.enabled = true
network.unix_sockets = { "/Users/detoro/Library/Application Support/Conclave/conclave.sock" = "allow" }
```

Why each line:
- `extends = ":workspace"` — inherit the built-in workspace-write profile (keeps out-of-workspace
  **exec** + writable roots; without it, test G failed with `execvp … Operation not permitted`).
- `network.enabled = true` — the unix-socket grant only takes effect with the network layer on
  (matches Marty's earlier probe). Note: this enables codex's **network permission subsystem**,
  not open internet — no domains are allowed, only the one socket.
- `network.unix_sockets = { "<sock>" = "allow" }` — the actual hole. Inline TOML table, quoted
  key (spaces + slashes fine).
- `default_permissions = "conclave"` — apply this profile to sandboxed tool calls. Alternatively
  select per-invocation with `-P conclave` / `--permissions-profile conclave` (CLI) if you'd
  rather not move the global default.

### codex open questions from the plan — resolved

1. **writable_roots covers socket connect?** No — unix sockets are a separate permission layer
   (`permissions.<name>.network.unix_sockets`), not `writable_roots`. **Confirmed.**
2. **`-c` override syntax for arrays/nested/tables?** Dotted path + TOML-parsed value; inline
   tables and quoted keys-with-spaces work. **Confirmed** (test J).
3. **out-of-workspace binary exec in workspace-write?** Allowed — only the **socket** was ever
   blocked. Dabin/Marty's failures were the socket, not the exec. **Confirmed** (test A).
4. **MCP-server fallback?** **Not needed** — direct config solves it; the MCP path is moot.

Profile system: live in codex 0.142.5. Built-in profiles seen: `:workspace` (**confirmed** works),
`:read-only`, `:danger-full-access` (named in docs — **plausible**, not exercised).

---

## claude-code — CONFIRMED from official settings JSON schema

Verified against the official schema (`https://www.schemastore.org/claude-code-settings.json`,
190 KB, fetched 2026-07-04) and the sandboxing docs
(`https://code.claude.com/docs/en/sandboxing`). Exact verbatim keys & types below.

Two layers matter and are distinct:
- **Permission prompt** ("allow this command?") → `permissions.*` / `autoAllowBashIfSandboxed`.
- **OS seatbelt modal** (the thing the human sees) → `sandbox.*`.

`permissions.allow` alone does **not** silence the seatbelt modal — the guide confirmed and the
schema agrees. You need a `sandbox.*` key for that.

Exact schema keys (all real, verbatim):

| Key (path under `sandbox`) | Type | Purpose |
|---|---|---|
| `sandbox.network.allowUnixSockets` | `string[]` | allow specific UDS paths; **defaults to blocking** |
| `sandbox.network.allowAllUnixSockets` | `bool` | allow all UDS (overrides the list) — broad, not recommended |
| `sandbox.excludedCommands` | `string[]` | commands that **never** run in the sandbox |
| `sandbox.autoAllowBashIfSandboxed` | `bool` | auto-approve bash **without prompting** when it runs sandboxed |
| `sandbox.allowUnsandboxedCommands` | `bool` | governs the `dangerouslyDisableSandbox` param (default true) |

### Recommended claude-code injection — Route A (surgical, mirrors codex)

Poke one hole for the socket; keep conclave fully sandboxed; auto-approve the sandboxed call:

```json
{
  "sandbox": {
    "network": {
      "allowUnixSockets": ["/Users/detoro/Library/Application Support/Conclave/conclave.sock"]
    },
    "autoAllowBashIfSandboxed": true
  }
}
```

- `allowUnixSockets` removes the seatbelt EPERM/modal for exactly that socket.
- `autoAllowBashIfSandboxed` removes the permission prompt for commands that stay sandboxed.
- This is the direct analogue of the codex `network.unix_sockets` fix.

### Route B (exclude the command from the sandbox)

```json
{
  "sandbox": { "excludedCommands": ["conclave"] },
  "permissions": { "allow": ["Bash(/Users/detoro/Library/Application Support/Conclave/bin/conclave *)"] }
}
```

- `excludedCommands` runs conclave **outside** the seatbelt → no socket restriction at all.
  Docs value format is bare command tokens/globs (`["git","docker"]`); the Conclave bin dir is
  first on `PATH`, so agents invoke bare `conclave` and `["conclave"]` matches. Add the absolute
  path too if any caller uses it.
- Because an excluded command runs **unsandboxed**, it needs `permissions.allow` (spaces in the
  path are fine, no escaping — confirmed) to skip the permission prompt.

**Prefer Route A:** it keeps conclave inside the sandbox and opens only the single IPC socket —
a strictly smaller hole than un-sandboxing the whole binary, and symmetric with the codex fix.

Injectable via per-agent `settings.json` (Conclave already writes these) or the `--settings` flag.

### Version

The earlier "needs v2.1.187+" note is **corrected**: v2.1.187 is the floor for `sandbox.credentials`
specifically. Core sandbox + `excludedCommands` + `network.allowUnixSockets` predate it. This
machine's **2.1.201** supports all keys above. Version is a non-blocker here.

---

## Confirmed vs inferred — honest ledger

**Confirmed (ran it / authoritative schema):**
- codex: EPERM cause, exec-allowed, and the full working `[permissions.conclave]` recipe — all
  reproduced under `codex sandbox` (tests A/B/C/J/K).
- claude-code: every key name, path, and type from the official settings JSON schema + docs.

**Inferred (high confidence, not exercised end-to-end):**
- codex: proven via `codex sandbox`; the live `codex exec` agent turn uses the same
  `default_permissions` seatbelt for its shell tool calls (per the key's own doc string
  "…applied to sandboxed tool calls"), so it should behave identically — **not** yet run through a
  real `codex exec` conversation. One-command check the lead can run:
  `codex exec -c 'permissions.conclave.extends=":workspace"' -c 'permissions.conclave.network.enabled=true' -c 'permissions.conclave.network.unix_sockets={"…conclave.sock"="allow"}' -c 'default_permissions="conclave"' 'run: conclave --version'`.
- claude-code: recipe is schema/docs-confirmed but **not** exercised in a live sandbox-enabled
  session (this research session runs sandbox-off under `bypassPermissions`, so the modal never
  fires here to reproduce against). Verifying it needs a sandboxed spawn with the settings above.

**Recommendation for the lead:** ship Route A (claude) + the `[permissions.conclave]` profile
(codex) as the spawn-time injection. Both are the same shape — one socket allowlisted, sandbox
otherwise intact, zero user action. If you want belt-and-suspenders before wiring it into the
spawner, run the two live checks noted above (codex `exec`, claude sandboxed spawn).
