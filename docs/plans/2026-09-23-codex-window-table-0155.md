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

## Amendment 1 (Detoro, 2026-09-23, after a44a01f was approved) — SUPERSEDES the Ruling section above where they conflict
Trigger: human direction "Context window 1M" with a screenshot of OpenAI's models page showing GPT-6 Astra / Sol / Luna each at a 1.05M context window. Plus two facts the original ruling got wrong:

- Codex clamps `-c model_context_window=N` to the catalogue `max_context_window` ITSELF (codex source `models-manager/src/model_info.rs`, tag rust-v0.153.4). The "launch path passes the table value unclamped, so auto-compact fires after the real cap" claim in §Why now is therefore wrong for gpt-5.5 (400K request → codex clamps to 272K, compact at min(380K, 0.9×272K)) and unproven for gpt-5.6 (372K is under its 872K catalogue max). `context_window` in `codex debug models` is the DEFAULT window, `max_context_window` is the cap. Do not cite §Why now's harm sentence anywhere.
- Live evidence, 2026-09-23 `ps`: every running Codex agent (gpt-6-astra ×3, gpt-5.6-sol, gpt-5.6-terra) launches with the Builder's 1M pair and Conclave shows 816,400 usable for all five = codex reporting 872,000 × 95 % − 12,000. So codex serves the 872K catalogue max for both families today, and the human runs everything at 1M.

Amended values for `codex_model_context_window`:
1. `gpt-6-astra | gpt-6-sol | gpt-6-luna` → `Some(872_000)`. This is the Codex-effective max for the human's "1M": the 1.05M API headline is clamped to the 0.155.1 catalogue `max_context_window` 872_000, so Auto now launches `model_context_window=872000 / model_auto_compact_token_limit=828400` and codex reports 828,400 usable — byte-identical runtime to choosing 1M in the Builder (codex takes min(cfg, 0.9 × 872K) = 784,800 for compaction either way). Comment must say: human direction 2026-09-23 "1M"; API 1.05M; codex catalogue max 872_000; live-verified 828,400 reported on 0.155.1.
2. `gpt-5.5` → `Some(272_000)` — keep a44a01f's change (catalogue max IS 272_000).
3. `gpt-5.6-sol | gpt-5.6-terra | gpt-5.6-luna` → RESTORE `Some(372_000)` (revert a44a01f's 272_000). Catalogue default 272K / max 872K; the July server-side ~372K cap is neither confirmed nor refuted by 0.155.1, and the human's agents run on the 1M pair anyway. Lowering to the default has no evidence behind it; raising to 872K is a separate decision needing a >372K live run. Update the arm comment: 0.155.1 catalogue numbers, "server cap still unverified", drop the issue-#31860 paragraph to one sentence.
4. gpt-5.4 family: unchanged, as ruled.
5. Mellow's readability nit on a44a01f: apply (gpt-5.5 arm above the absent-from-catalogue comment).

Tests: `known_models_resolve_documented_max` (astra/sol/luna 872_000, 5.5 272_000), `gpt_5_6_family_*` back to 372_000, the Auto-on-astra usable test near codex_models.rs:326 becomes 872_000 × 95 % − 12_000 = 816_400 (same as the 1m case — assert both and say why), keep a clamp-exercising case with a fixture max below the table, and the new `append_codex_context_window_config` test pins gpt-6-sol Auto → `model_context_window=872000` / `model_auto_compact_token_limit=828400`. Also update the Builder hint text in `src/components/builder/RuntimeSection.tsx` ONLY if it still says "GPT-5.6 ≈ 372K" in a way that becomes false — it does not (372K stays), so leave it.

Process: Dew continues on lane/codex-window-table-0155 with new commits on top of a44a01f (do not restart from main); re-run all gates; Mellow re-reviews the lane tip; Detoro merges.
