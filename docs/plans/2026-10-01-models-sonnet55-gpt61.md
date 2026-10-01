# Add claude-sonnet-5-5, gpt-6.1-sol (and confirm gpt-6-luna) to the model presets
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop
Implementer: Dew (60ff2775-14a2-4db4-ab44-6df5bb13bf2a), allocated by Detoro. Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f). Detoro rules and merges. Start lane from main at or after 242f9f0.

## Why now
Human request 2026-10-01: "support model sonnet-5-5, gpt-6.1-sol, gpt-6-luna". Facts Detoro verified:
- `gpt-6-luna` is ALREADY in both preset lists and in `codex_model_context_window` (872_000) since 242f9f0 — nothing to add; confirm in the READY note and move on.
- `claude-sonnet-5-5` is the real id of Claude Sonnet 5.5 (1M context, same tokenizer/price tier as Sonnet 5). It is absent from `CLAUDE_MODELS`.
- `gpt-6.1-sol` is absent from `CODEX_MODELS` AND from the codex-cli 0.158.0 catalogue (`codex debug models`, 2026-10-01: lists gpt-6-astra/sol/luna, gpt-5.6-sol/terra/luna, gpt-5.5; hidden gpt-reserve, codex-auto-review). The human asked for it anyway; the Builder lets the user type any id, so the preset is a convenience, and the window entry only seeds the meter.

## Rulings (final)
R1. `CLAUDE_MODELS` (both mirrors) inserts `"claude-sonnet-5-5"` immediately after `"claude-opus-5-5"` (newest-first within the 5.x generation, same precedent as the 2026-09-23 opus-5-5 add — see bb key `lead:why-solo:add-claude-opus-5-5`).
R2. `CODEX_MODELS` (both mirrors) inserts `"gpt-6.1-sol"` at index 0 (newest family first; astra/sol/luna keep their 0.155.1 catalogue order after it).
R3. `codex_model_context_window`: add `"gpt-6.1-sol" => Some(872_000)` in its own arm ABOVE the GPT-6 arm, with this comment (verbatim intent): human request 2026-10-01; NOT in the codex-cli 0.158.0 catalogue as of 2026-10-01 so no measured max; 872_000 is the GPT-6 family cap and codex clamps any over-request to its own `max_context_window` (model_info.rs), so a too-high seed is harmless — re-derive at the first codex release that lists it. Also update the file-header "re-check at the next bump" sentence to name 0.158.0 as the last checked version.
R4. `claude_model_context_window` in `transcript_context.rs` is NOT touched: sonnet-5-5 behaves like sonnet-5 there (1M only via the `[1m]` Builder choice; default stays the session fallback). Same treatment opus-5-5 got.
R5. No display-name/logo mapping exists for models (grep `gpt-6-astra` in `src/` hits only `modelCatalogue.ts`), so nothing else to mirror.

## Files (boundary)
- `src/lib/modelCatalogue.ts` — R1, R2.
- `src-tauri/src/engine/commands/draft.rs` — R1, R2 mirrors + doc comments ("Human request 2026-10-01: add Sonnet 5.5 after Opus 5.5" / "add gpt-6.1-sol first") + tests: the mirror tests at ~979-1017 (`CLAUDE_MODELS.get(1) == opus-5-5` still true; add `.get(2) == "claude-sonnet-5-5"`; `CODEX_MODELS.first() == "gpt-6.1-sol"`, astra/sol/luna shift to 1/2/3) and the loop at ~1021-1024 gains `gpt-6.1-sol`.
- `src-tauri/src/engine/codex_models.rs` — R3 + a test assert (`codex_model_context_window("gpt-6.1-sol") == Some(872_000)`) in the existing `known_models_resolve_documented_max`-style test (~365).
Nothing else. `draft_prompt.rs` test needles (`claude-sonnet-5`, `gpt-5.5`) remain satisfied by substring.

## Steps
1. `conclave lane start 11ecf99b-53f4-4c24-b538-b19e5933a9e3 models-sonnet55-gpt61`; `pnpm install` once in the worktree.
2. Edit the three files per R1-R3; keep the TS and Rust arrays byte-identical in order (the Rust test parses the TS file).
3. Gates (record each with `conclave task gate <ws> models-sonnet55-gpt61 -- <cmd>`):
   - `cargo test --manifest-path src-tauri/Cargo.toml --quiet draft`
   - `cargo test --manifest-path src-tauri/Cargo.toml --quiet codex_models`
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml --quiet --all-targets -- -D warnings`
   - `pnpm tsc --noEmit`
   - UI Pixel Gate (CLAUDE.md standing protocol — the Builder's model picker renders these presets): `pnpm uishot builder --viewport 1440x1900`, then READ the PNG and confirm `claude-sonnet-5-5` and `gpt-6.1-sol` are visible in the picker; attach the shot path in the READY note. Kill any foreign vite server on :1420 first (`lsof -nP -iTCP:1420 -sTCP:LISTEN`).
4. Run `codex debug models 2>/dev/null | python3 -c 'import json,sys; [print(m["slug"], m["context_window"], m["max_context_window"]) for m in json.load(sys.stdin)["models"]]'` yourself and paste the lines (plus `codex --version`) into the READY note — your own reading, not Detoro's.

## Risk ledger
- If a newer codex-cli than 0.158.0 is installed by the time you run step 4 and it DOES list gpt-6.1-sol with a max below 872_000, do not silently change the value: file a `task challenge` with the catalogue line; default = keep 872_000 (codex clamps).
- The uishot `builder` view with `--full` clips the modal; use `--viewport 1440x1900` as written.

## Done means
One commit on `lane/models-sonnet55-gpt61` (`git commit -- <the three paths>`), gates recorded, READY note with SHA, shot path, catalogue excerpt, and the gpt-6-luna "already present" confirmation. Mellow reviews; Detoro merges. Needs app rebuild + relaunch to show in the live Builder.

## Amendment 1 (Detoro, 2026-10-01, after ab91501 READY) — gpt-6.1-sol now has a measured catalogue entry
Tiësto's codex probe on the other lane accidentally accepted codex's self-update dialog: global codex-cli went 0.158.0 → 0.159.3 (`~/.codex/packages/standalone/current`). Detoro ruled to KEEP 0.159.3: codex auto-updates on this machine anyway (`standalone/auto-update-version`, 18 releases since June), and 0.159.3 is the first catalogue that lists the model the human asked for. `codex debug models` on 0.159.3 (Detoro, 2026-10-01): `gpt-6.1-sol 272000 / 872000 list` at the top, every other row unchanged from 0.158.0.
R3'. The `gpt-6.1-sol` arm keeps `Some(872_000)`; rewrite its comment to the measured fact: "codex-cli 0.159.3 catalogue (2026-10-01): context_window 272000 (default) / max_context_window 872000 — same Codex-effective cap as the GPT-6 family (codex clamps to max_context_window)." Drop the "not in the catalogue / no measured max / harmless over-request" sentences. The file-header "Last checked" line says 0.159.3. The `CODEX_MODELS` doc comment in draft.rs drops "not yet in the codex-cli 0.158.0 catalogue" (say "first listed in codex-cli 0.159.3"). No value or order changes.
Process: Dew adds one commit on top of ab91501, re-runs the Rust gates (test draft, test codex_models, fmt, clippy; tsc/uishot unaffected by comment-only Rust edits, do not re-run), re-posts READY with the new SHA and the 0.159.3 catalogue line from `codex --version` + `codex debug models` run by Dew. Mellow reviews the new tip.
