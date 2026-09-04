# Antigravity authenticated-model dropdown

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Replace Builder's Antigravity free-text model field with a dropdown populated from the models available to the user's authenticated Google Antigravity CLI. Preserve the existing blank model as `Auto` so Antigravity can still choose its default.

The user's last sentence was truncated after `Replac`. The settled in-loop interpretation is to replace the existing free-text model input, not the Antigravity harness, effort control, or execution-mode control. This is the narrowest complete feature consistent with the explicit dropdown request and the current product.

## Design and behavior decisions

- Keep `model?: string` persistence and `agy --model <id>` launch behavior unchanged. This task changes discovery and selection, not the stored schema or launch contract.
- Add read-only IPC command `instance.cliModels` with request `{ cliKind: "antigravity" }` and response `{ models: Array<{ id: string; label: string }> }`.
- The backend maps the allowlisted `antigravity` kind to the fixed executable `agy`; the renderer cannot provide a binary name or path.
- Run `agy models` through the same login + interactive shell used by `instance.cliStatus` and real launches. Parse stdout rows as tab-separated `id` and human label; ignore blank rows; reject a successful response that contains no valid models. On a missing label, fall back to the id. Ignore Antigravity's success progress line on stderr.
- Bound the external catalog query to 15 seconds and ensure a timed-out child is killed on drop. A timeout is the same retryable query-error class as a non-zero exit; Builder must not remain permanently disabled when auth or the network stalls.
- A non-zero `agy models` exit and a shell-spawn failure are query errors, not "agy missing". Surface a concise retryable UI state without exposing raw shell output as UI copy.
- Query models only after `instance.cliStatus` reports `available: true`. `Check again` retries availability and model discovery.
- The Antigravity model control is a familiar native `<select>` styled like the existing Execution mode select. Its first option is `Auto (authenticated default)` with value `""`; remaining options use the returned display label, with the exact model id available in the option copy/title where practical.
- While loading, disable the selector and show a loading option/hint. If discovery fails, retain `Auto`, show concise error text plus a retry action, and do not block saving an Auto configuration.
- Editing must be lossless: when a nonblank saved/drafted model is absent from the latest catalog, inject one selected option labelled as the current unavailable model. Do not silently clear it. Once the user chooses Auto or a returned model, normal selection applies.
- Do not hardcode production model IDs. Fixture rows are fixed deterministic test data only.
- The accepted Antigravity visual canon remains `design/screens/antigravity-cli.tsx` at `78c98058fb6e6eb2b3cecf59fe74ebb505203834`; this plan supersedes only that canon's free-text model-field behavior per the user's new ruling. Preserve all other visual and behavioral choices.

## Exact implementation paths

- `src-tauri/src/engine/commands/instance.rs`
  - Add the allowlisted model-list query helper and parser.
  - Export the `instance.cliModels` handler.
  - Add focused async/unit tests for command construction, successful parsing, empty/malformed success, non-zero exit, timeout, and unsupported `cliKind`.
- `src-tauri/src/engine/router.rs`
  - Route `instance.cliModels`.
- `src/ipc/commands.ts`
  - Add the typed request/response contract and `ipc.instance.cliModels` wrapper.
- `src/components/Builder.tsx`
  - Add model-catalog state/loading/retry logic and replace only the Antigravity free-text field with the dropdown. Claude Code and Codex model behavior must remain unchanged.
- `src/fixtures/scenarios/default.ts`
  - Add a deterministic successful model catalog.
- `src/fixtures/scenarios/empty.ts`
  - Add deterministic empty/missing-CLI coverage so fixture mode never hits a missing handler.

## Verification

Run from the lane checkout:

1. `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
2. `cargo test --manifest-path src-tauri/Cargo.toml antigravity_cli_models`
3. `pnpm build`
4. Before the UI shot, run `lsof -nP -iTCP:1420 -sTCP:LISTEN`; if a server exists, verify its cwd is this lane checkout and stop only a foreign/stale server.
5. `pnpm uishot builder`
6. Open and inspect `.shots/builder-default.png` with an image-capable reader. Also interactively select the Antigravity segment and capture/inspect a supplemental screenshot of the loaded dropdown if the fixed uishot route still opens Claude Code by default.
7. Record the required pixel gate with `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 antigravity-model-dropdown -- pnpm uishot builder` and attach all inspected shot paths in the READY note.

Expected: build/test commands exit 0; Builder preserves its existing density and width; Antigravity shows a keyboard-focusable dropdown with Auto plus fixture model choices; loading/error/current-unavailable copy does not overflow; Claude/Codex views are visually unchanged.

## Risk ledger

- `agy models` is network/auth dependent and can fail or stall while the executable itself is installed. Keep availability and catalog failures distinct, and bound the query so the loading state always settles.
- Model availability changes over time. Dynamic data and lossless stale-value handling are required; a baked-in production list is incorrect.
- Login-shell execution is a security boundary. Only the backend-selected literal `agy models` command is allowed; never interpolate renderer-controlled executable text.
- Native select labels can be long. Constrain width to the existing Builder row, preserve the exact selected id in accessible/title copy, and visually inspect truncation.
- Fixture handlers are loud by design. Both default and empty scenarios must cover the new render-path IPC command.
- The Builder uishot opens Claude Code by default. The standing pixel gate is still required, plus a supplemental inspected Antigravity-state capture so the changed pixels are actually seen.
