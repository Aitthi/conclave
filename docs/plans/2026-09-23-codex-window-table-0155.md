# Re-derive the Codex context-window table from codex-cli 0.155.1
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop
Implementer: Dew (60ff2775-14a2-4db4-ab44-6df5bb13bf2a), allocated by Detoro. Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f). Detoro rules and merges. Start lane from main at or after f16a52b.

## Why now
`codex_model_context_window` (src-tauri/src/engine/codex_models.rs) carries values live-verified on codex-cli 0.144.1 / 0.153.2, and its own comment says "re-check at the next Codex CLI version bump". 0.155.1 is that bump. Mellow (review of f16a52b, 2026-09-23) and Detoro both ran `codex debug models` on 0.155.1 and the catalogue now reports:

| slug | context_window | max_context_window | table today |
|---|---|---|---|
| gpt-6-astra / gpt-6-sol / gpt-6-luna | 272000 | 872000 | 272_000 (correct) |
| gpt-5.6-sol / terra / luna | 272000 | 872000 | 372_000 (TOO HIGH) |
| gpt-5.5 | 272000 | 272000 | 400_000 (TOO HIGH) |
| gpt-5.4, gpt-5.4-mini, gpt-5-codex, gpt-5.3-codex, gpt-5.3-codex-spark | absent from the 0.155.1 catalogue | unchanged |

Harm path (confirmed by reading the code, not speculative): `append_codex_context_window_config` (commands/instance.rs:75) passes the TABLE value straight to `-c model_context_window=` and 95% of it to `-c model_auto_compact_token_limit=` with no catalogue clamp. On gpt-5.6 that launches codex with 372000 / 353400 against a 272000 served window, so auto-compact can only fire after the real cap — exactly the failure the 2026-07-11 ruling (challenge 89599d2e) said must not happen. gpt-5.5 launches with 400000 / 380000, same class. The meter seed (`codex_usable_context_window`) is saved by its `max_context_window` clamp; the launch path is not.

## Ruling (Detoro, 2026-09-23)
1. Set `gpt-5.6-sol | gpt-5.6-terra | gpt-5.6-luna` to `Some(272_000)` and `gpt-5.5` to `Some(272_000)`. Cite codex-cli 0.155.1 `codex debug models`, dated 2026-09-23, in the arm comments; keep the history of the 372_000 / 400_000 values in the comment as one sentence each, not a paragraph.
2. Leave `gpt-5.4`, `gpt-5.4-mini`, `gpt-5-codex`, `gpt-5.3-codex`, `gpt-5.3-codex-spark` values untouched. They are absent from the 0.155.1 catalogue so there is no fresh evidence either way; add one comment line above that group saying so with the date. Do NOT remove them from the table or from CODEX_MODELS — existing agent rows reference them.
3. No launch-side clamp in this task. The table IS the served value by definition; a defensive clamp is separate hardening and needs its own repro (standing rule: no speculative hardening).
4. Do not touch CODEX_MODELS, draft.rs, modelCatalogue.ts, or instance.rs.

## Tests
- Update `known_models_resolve_documented_max`, `gpt_5_6_family_resolves_codex_enforced_ceiling` (rename to say 0.155.1 served window), and the `\tgpt-5.5\n` trim assertion to the new values.
- The `codex_usable_context_window` test around codex_models.rs:334 ("table 400_000 clamped to the catalog cap 272_000") must be re-derived: with the table now 272_000 the clamp is a no-op there. Keep a clamp-exercising case by using a fixture catalogue whose max is BELOW the table value for some model (e.g. gpt-5.4 with max 500000), so the clamp path stays covered.
- Add one test asserting `append_codex_context_window_config` on gpt-5.6-sol Auto emits `model_context_window=272000` and `model_auto_compact_token_limit=258400` — this pins the actual harm path. If that requires touching instance.rs tests, that single test block in instance.rs is inside the boundary.

## Gates (record with `conclave task gate`, not prose)
- `cargo fmt --check` (in src-tauri)
- `cargo test --quiet codex_models`
- `cargo test --quiet append_codex_context_window_config` (or the instance test filter you used)
- `cargo clippy --quiet --all-targets -- -D warnings` only if it was green on main before your change; otherwise note the pre-existing state.
No UI change, so no uishot gate.

## Done means
Commit(s) on the lane with the table + tests + comments, gates recorded, task note READY with the live `codex debug models` excerpt (slug / context_window / max) you verified yourself — do not reuse Detoro's or Mellow's reading. Mellow reviews; Detoro merges to main.
