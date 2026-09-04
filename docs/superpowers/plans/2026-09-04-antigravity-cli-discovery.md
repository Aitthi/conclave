# Antigravity CLI support discovery

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Map the smallest complete, production-safe change for Conclave to support Google Antigravity CLI as a first-class `cliKind`, alongside Claude Code and Codex.

## User evidence and verified environment

- Screenshot: `/Users/detoro/Desktop/Screenshot 2569-09-04 at 20.48.00.png`
- Installed executable: `/Users/detoro/.local/bin/agy`
- Installed version: `agy 1.1.26`
- Interactive invocation: `agy`
- Verified flags include `--model`, `--effort low|medium|high`, `--agent`, `--mode accept-edits|plan`, `--continue`, `--conversation <uuid>`, `--prompt-interactive`, `--dangerously-skip-permissions`, and stream-json print mode.
- Current authenticated model list includes Gemini 3.8/3.7/3.6 Flash effort variants, Gemini 3.1 Pro, Claude Sonnet/Opus 4.6, and GPT-OSS 120B.

Primary sources:

- https://github.com/google-antigravity/antigravity-cli
- https://www.antigravity.google/docs/cli/using/
- https://github.com/GoogleCloudPlatform/evalbench/blob/main/docs/agy_cli_agent_testing.md

## Research scope

- Read-only trace of schema constraints, agent definition CRUD, Builder choices/model controls, CLI launch construction, PTY lifecycle, prompt injection, context/transcript metering, skill/role instruction delivery, rtk hook behavior, sandbox/permissions, resume/restart, fixtures, packaging/detection, and tests.
- Run only read-only local probes (`agy --help`, `agy --version`, model/agent listings, help subcommands). Do not start an interactive agent or mutate Antigravity settings/auth.
- Determine whether support should be a new `antigravity` kind using executable `agy` or reuse `custom`; default model/effort semantics; safe permission flag behavior; how Conclave instructions reach the CLI; context limit/meter behavior; session resume contract; and platform executable naming.
- Account for migration ordering: workspace lifecycle is adding v27 concurrently, so this feature must plan v28 or a later available version after integration.

## Deliverable

Attach one bounded READY note with confirmed behavior, exact files/symbols, schema/migration needs, UI contract, launch argv, security defaults, unsupported/deferred capabilities, tests, gates, risks, and a suggested implementation decomposition. No product edits.
