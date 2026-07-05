# Plan: Design craft — anti-slop skills, review gate, evals (feature `design-craft`)

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro, lead) · authority: in-loop
requested by the human 2026-07-05 ~17:2x: "ต้องการความสามารถในการ Design ออกมาให้ไม่เหมือน AI ทำ ของ arta ทั้งหมดด้วยนะ เช่น /Users/detoro/code/arta/skills/arta, evals, slop detect" — port ALL of Arta's design-quality capability into the design-native system.

## Source material (read-only; the arta repo is NOT part of this workspace)

- `/Users/detoro/code/arta/skills/arta/SKILL.md` (70KB) + `component-cookbook.md` + `design-systems.md` + `reference/` (14 craft docs incl. critique-rubric)
- `/Users/detoro/code/arta/mcp/slop-detect.mjs` (+ its `.test.mjs`) — dependency-free deterministic anti-slop detector, JSX-aware (`detectSlopJsx`); ported from Hallmark's MIT gate set — KEEP the attribution comment
- `/Users/detoro/code/arta/evals/` — `grade.mjs` (6 deterministic assertions A1a..A5), `gate.mjs`, `thresholds.json`, `briefs.json`, fixtures

Key fit: arta's grader reads `screens/*.tsx` + `components/*.tsx` + `theme.css` straight off disk — the same shape as `<workspace>/design/`. AMENDED (Tiësto, at claim): "re-point, not a rewrite" was optimistic — grade.mjs imports arta-internal helpers (`src/lib/prototype.ts::tokensFromCss`, `vite/proto-manifest.ts` listScreens/listComponents/readConfig, `evals/briefs.json`) that assume the `proto/` subdir shape; porting means vendoring/inlining those helpers into `design-host/review/`. Accepted as implementation judgment.

## Lead rulings (challenge via `task challenge` if wrong)

- **R1 — Skills are rewritten in place; ids/dir names never change.** `design-canvas` and `arta-designer` (src-tauri/skills/) teach the dead `.arta` contract — their CONTENT is rewritten for the native `design/` contract; their dir names stay (role definitions reference skill ids; `repo::skill::list_builtin` scans the folder, so a NEW skill = a new dir, no Rust edit).
- **R2 — One new `design-craft` skill, self-contained.** Distill the craft core (anti-slop vocabulary, token discipline, critique-rubric essentials, layout/type/color rules) into ONE SKILL.md ≤ ~10KB. Do NOT inject 70KB into agent context; do NOT copy the 14 reference docs in v1 — a workspace `design/craft/` reference library is DEFERRED (recorded here, cut only if asked).
- **R3 — Review runs as a CLI gate, not an overlay.** `conclave design review <workspaceId> [--json]` → engine `design.review` → spawns node (same login-shell resolution as the host) running the ported grader against `<workspace>/design/`. Exit 0 = no serious findings; agents use it as a `task gate`. A live viewer overlay (arta_design_review-style) is DEFERRED.
- **R4 — Grader semantics adapt to `design/`:** A1a/A1b (tokens defined/used) run against `design/theme.css` — scaffold gains a minimal `theme.css` so a fresh workspace passes; A2 (shared layout) unchanged over `design/components/`; A3 (nav reachability) runs ONLY when `design/config.json` with `{"start": "<screen>"}` exists — absent = skipped, and the scaffold does NOT create it; A4 (valid TSX + default export) unchanged (esbuild comes with the host's node_modules); A5 = `detectSlopJsx` 0 serious findings. Finding shape `{ antipattern, severity, file, line, snippet, message }` is pinned.
- **R6 — Host reaches Arta authoring parity** (ruled on Arta's cross-lane flag
  at Lane S review): the skills teach the Arta-parity authoring contract —
  curated imports `{react, react-dom, react-router-dom, motion, lucide-react,
  recharts, clsx, tailwind-merge}` + styling via Tailwind v4 utilities with
  tokens in `design/theme.css` (`@theme` block, `@source` for workspace dirs) —
  so `design-host` must provide exactly that set: alias/dedupe the full curated
  list in vite.config.ts and wire `@tailwindcss/vite` compiling the workspace's
  `design/theme.css`. The R4 scaffold theme.css carries a real `@theme` token
  set the welcome screen uses. Rationale: the human asked for ALL of arta's
  capability; grader A1a already expects `@theme` tokens; skills that promise
  imports the host can't resolve produce broken-by-construction designs.
  REJECTED: reduced react-only contract. Lands in Lane R (item 6). Credit:
  Arta for catching the gap before it shipped.
- **R5 — Evals = Layer A only (deterministic regression gate), local.** Port `gate.mjs`/`thresholds.json` + two fixtures (one good `design/`, one bad) into `design-host/evals/`; runnable locally and as a task gate. The repo has no CI workflows — do not add one. Layer B (LLM builder loop) is DEFERRED.

## Lanes (independent; lead integrates)

### Lane S — `craft-skills` (Arta): src-tauri/skills only
1. Rewrite `src-tauri/skills/design-canvas/SKILL.md`: the `design/` contract — `screens/*.tsx` + `components/*.tsx` + `lib/` + optional `theme.css`/`config.json`, HMR host renders live, written with ordinary file tools; remove every `.arta`/`state.json`/viewer-section mention.
2. Rewrite `src-tauri/skills/arta-designer/SKILL.md`: designer-role behavior on the native canvas (brainstorm → screens → iterate on feedback → hand off recorded design), composing with `design-craft`; remove Arta plugin/MCP/localhost:7317 mechanics.
3. NEW `src-tauri/skills/design-craft/SKILL.md`: the distilled craft core per R2 — what makes design NOT look AI-made: the slop vocabulary (gradient text, side-stripe borders, nested cards, transition-all, uniform hover scale, emoji-as-icon, italic headings, placeholder names, mixed icon libs, cramped tracking…mirror slop-detect's gates so prose and detector agree), token discipline, spacing/type/color/layout rules, and the critique rubric distilled from `reference/critique-rubric.md`. End with: run `conclave design review <ws>` before calling a design done.
Boundary: `src-tauri/skills/design-canvas/SKILL.md, src-tauri/skills/arta-designer/SKILL.md, src-tauri/skills/design-craft`
Gate: `cd src-tauri && cargo test --lib` (skill loader tests scan the folder).

### Lane R — `design-review` (engine implementer): detector + grader + CLI + evals
1. Vendor `design-host/review/slop-detect.mjs` (verbatim port incl. tests as `slop-detect.test.mjs`, MIT attribution kept) and `design-host/review/grade.mjs` (arta's grade.mjs re-pointed at a `design/` root per R4).
2. `design-host/review/review.mjs` — entry: `node review.mjs <designDir> [--json]`, prints per-assertion table or JSON, exit non-zero on any serious finding / failed assertion.
3. Engine: `design.review { workspaceId }` in `commands/design.rs` → resolves the workspace `design/` dir, spawns node review.mjs, returns `{ pass, findings, assertions }`. CLI verb `conclave design review <workspaceId> [--json]` in `commands/cli.rs`. Router registration one-arm additive (choke files — semantic-diff guard).
4. Scaffold: add minimal `design/theme.css` to `scaffold_if_missing` (R4) — tokens the welcome screen actually uses.
5. Evals per R5: `design-host/evals/{gate.mjs,thresholds.json,fixtures/good,fixtures/bad}` — gate greps the grader over both fixtures vs thresholds; wire nothing into CI.
6. Host authoring parity per R6: extend `CURATED` + aliases in `design-host/vite.config.ts` to the full set, add the deps + `@tailwindcss/vite` to design-host/package.json, compile the workspace `design/theme.css` (Tailwind v4 `@theme`/`@source`); scaffold theme.css (item 4) gains a real token set. The old `.arta` viewer solved the same wiring — its pre-swap `THEME_CSS` template (git history of `commands/design.rs`) is the reference.
Boundary: `design-host, src-tauri/src/engine/commands/design.rs, src-tauri/src/engine/commands/cli.rs, src-tauri/src/engine/router.rs, src-tauri/src/engine/runtime/design_host.rs`
Gates: `cargo test --lib` + `cargo clippy --all-targets -- -D warnings` + `node design-host/review/slop-detect.test.mjs` + `node design-host/evals/gate.mjs`.

## Global constraints

- Shared checkout: `stage commit` only; choke-point semantic-diff guard on `design.rs`/`cli.rs`/`router.rs`.
- Skill prose is English (app-shipped content).
- The two lanes share NO files. Lane S's SKILL.md may reference the CLI verb Lane R builds (`conclave design review`) — the verb name above is the pinned interface; neither lane changes it without a ruling.
- Live checks → r13 human checklist per 35968ae3.

## Risk ledger

- `list_builtin` behavior with a new dir is assumed scan-based (fixture-driven tests suggest it) — Lane S verifies by running the skill-loader tests; if a hardcoded list exists somewhere after all, STOP and escalate.
- arta's grade.mjs A4 uses `esbuild.transform` — confirm esbuild resolves from design-host/node_modules (vite dep); if not, add it as an explicit devDependency of design-host (in-boundary).
- slop-detect gate list vs SKILL.md prose must AGREE (R2/Lane S item 3) — reviewer checks the two lanes side by side at LAND.
- The stale `arta-designer` skill is live in r12 preambles until the next rebuild — nothing to do now; r13 picks up the rewrite.
