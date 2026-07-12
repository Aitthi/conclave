# Compact: confirm popover (shadcn base) + drop auto clear/restore

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Goal (human's words, translated)

Fix the snapshot system in the bottom context bar: clicking the compact (⚡ Zap)
button must open a **shadcn base popover** (https://ui.shadcn.com/docs/components/base/popover)
to confirm, and once the compact (handoff save) completes, the engine must **no
longer auto `/clear` and auto-restore** the agent. Compact becomes a
non-destructive "checkpoint now" action; the human decides when/if to clear.

Human request 2026-07-12 (screenshot: bottom bar "Snapshots 6 · last handoff
~1,039 tok · 10m · ⚡ · 19%"). All UI copy stays **English** (standing rule).

## Settled decisions (Detoro rulings — do not re-open)

- **D1 Popover tech**: add dependency `@base-ui-components/react` and create
  `src/components/ui/popover.tsx` adapted from the shadcn *base* popover doc
  (Popover.Root / Trigger / Portal / Positioner / Popup). Style with THIS
  repo's Tailwind tokens (`bg-surface`, `ring-hair`, `text-text-secondary`,
  `rounded-lg shadow-lg`, 11–12px type), NOT shadcn CSS variables — the repo
  has no shadcn theme. Rejected: hand-rolled div popover (human explicitly
  linked the shadcn base component; Base UI gives outside-click/Escape/focus
  handling for free).
- **D2 Compact semantics**: `snapshot.compact` now = inject the save prompt
  only. The destructive tail (`/clear` + restore injection) is REMOVED, along
  with the whole compact-arming machinery. The **restart** flow
  (`restart_pending`, `run_respawn_resume`, `conclave restart`) is untouched —
  it still kills+respawns after save. `snapshot.resume` untouched.
- **D3 Prompt copy**: `compact_save_prompt` must stop claiming "your context is
  about to be cleared" (it no longer is). New contract: checkpoint now, then
  CONTINUE working. `compact_restore_prompt` becomes dead → delete it.
- **D4 Confirm scope**: popover confirm applies to the CLI-agent compact path
  only. For chat-type agents the ⚡ button keeps creating a manual snapshot
  instantly (non-destructive, cheap — no confirm).

## Files & exact edits

### 1) `package.json` / `pnpm-lock.yaml`
`pnpm add @base-ui-components/react` (run inside YOUR lane worktree; fresh
worktrees need their own `pnpm install` first — known gotcha).

### 2) NEW `src/components/ui/popover.tsx`
Adapted from shadcn base popover: export `Popover`, `PopoverTrigger`,
`PopoverContent` (wrap `Popover.Portal > Popover.Positioner > Popover.Popup`),
plus optional `PopoverTitle`/`PopoverDescription`. `PopoverContent` accepts
`side`/`align`/`sideOffset` and forwards to the Positioner. Keep it small
(<100 lines), typed, no `cn` util — template-literal classes like the rest of
the repo.

### 3) `src/components/ContextBars.tsx` (component `ContextBottomBar`, ~lines 470–745)
- Wrap the ⚡ button (lines ~678–687) as the popover trigger for the CLI case.
  Clicking it (cli) opens the confirm popover anchored to the button, opening
  UPWARD (`side="top"`, the bar is at screen bottom). Chat case: keep calling
  `doSnapshot()` directly (D4).
- Popover content: title **"Compact — save a handoff"**, description **"The
  agent will write a handoff snapshot of its work. It will NOT be cleared or
  restored automatically."**, buttons **[Save handoff]** (accent) →
  `doCompact()` + close, **[Cancel]** → close.
- DELETE the old inline confirm: `confirming` state (lines 482, 491, 518,
  636–639) and the `{confirming && …}` block in the snapshots panel (lines
  ~713–735). `onCompactClick` no longer opens the snapshots panel.
- Update ⚡ tooltip (line 681) → `"Compact: ask the agent to save a handoff
  snapshot"` (cli) / unchanged for chat.
- Update the compacting hint (lines ~736–741) → `"Asking the agent to save its
  handoff — watch its terminal."` (drop "then clearing & restoring").
- **Compacting pulse end**: previously the pulse effectively ran until the 120s
  failsafe. Now define success = a NEW handoff snapshot appears: when starting
  compact, record the current snapshot ids (`useRef<Set<string>>`); add an
  effect — if `compacting` and `snapshots.snapshots` contains a
  `type === "handoff"` row whose id is not in the recorded set →
  `setCompacting(false)`. Keep the 120s failsafe as-is.

### 4) `src-tauri/src/engine/commands/snapshot.rs`
- `compact()` (~line 423): remove `state.mark_compact_pending(...)`; keep the
  existence/liveness guards and the prompt injection. Rewrite the doc comment:
  compact injects the save prompt; nothing destructive follows; the SAFETY
  paragraph about gated clear goes away.
- `save()` (~line 375): delete the `else if state.take_compact_pending(...)`
  branch (KEEP the `take_restart_pending` branch verbatim).
- Delete `run_clear_restore`, `resolve_clear_cmd`, `clear_command`,
  `COMPACT_SETTLE_MS` (verify no other callers first — `submit_line` STAYS, the
  restart loop in `commands::instance` uses it). Update the module doc (lines
  ~5–10) which describes the clear/restore loop.
- Tests: delete `compact_pending_arm_is_consumed_once` (~line 830); in the
  restart-arm test (~841–851) drop the "not consumable as a compact" half but
  KEEP restart consume-once assertions; `compact_not_running_not_found` stays
  (assert it still spawns nothing).

### 5) `src-tauri/src/engine/state.rs`
Remove the `compact_pending` map (~lines 68–80, 105, 222) and
`mark_compact_pending`/`take_compact_pending` (~153–166). Keep
`restart_pending` and its TTL machinery untouched. Update the TTL doc comment
(~line 15) if it names compacts.

### 6) `src-tauri/src/engine/agentctx.rs`
- Rewrite `compact_save_prompt` (line 113). Keep the rich-handoff instructions
  (seven sections, 10k cap, reference-don't-paste, redact secrets, persist via
  `conclave snapshot save …`) but replace the framing sentence and the ending:
  - Open: `[conclave compact] Checkpoint your context NOW (human-triggered).` …
  - End: `After it confirms, tell the human in one line that the handoff is
    saved, then CONTINUE your current work — your context will NOT be cleared
    automatically; the snapshot is a restore point for later.`
- Delete `compact_restore_prompt` (line 131) and its test references (~lines
  837–849 keep only the save-prompt assertions; ~line 1012 remove).
  `resume_restore_prompt` and `restart_save_prompt` stay.

### 7) `src/ipc/commands.ts`
Update the `snapshot.compact` comment block (~lines 231–239): it no longer
"drives the whole loop"; it asks the agent to save a handoff, nothing follows.

## Gates (run each via `conclave task gate <ws> compact-confirm-popover -- <cmd>`)

1. `cargo test --manifest-path src-tauri/Cargo.toml snapshot` (or the engine
   test filter that covers `commands::snapshot`, `state`, `agentctx`).
2. `pnpm exec tsc --noEmit` (or `pnpm build`).
3. `pnpm uishot home` — the bottom bar lives in `WorkspacePane`
   (src/components/WorkspacePane.tsx:487/521/543). Confirm which uishot view
   renders it (`home` expected); **OPEN the PNG with your Read tool** per the
   UI Pixel Gate, attach the path in the READY note. Kill foreign :1420
   servers first (`lsof -nP -iTCP:1420 -sTCP:LISTEN`).

The popover-open state can't be captured by uishot (no click) — acceptable;
the pixel gate covers the bar's closed state; Detoro will exercise the popover
at integration.

## Risk ledger

- `@tauri-apps/api` getters can throw synchronously in plain Chrome — do not
  add any to the popover render path (fixture caveat, CLAUDE.md).
- uishot exits 1 on ANY console error; a missing fixture handler throws loudly
  — this change adds no new IPC calls, so none should be needed.
- `snapshot.rs` `save()` ordering comment (lines 359–366) explains why the tail
  fires from `save` — after removing the compact branch, rewrite that comment
  for restart-only so it doesn't mislead.
- Design canon: no `.arta` proto exists for this bar; canon = the shadcn base
  popover doc URL above, adapted to repo tokens. Design escalation: Detoro.

## Integration notes (lead-owned, outside the lane boundary)

- `src-tauri/src/engine/commands/instance.rs:38-41` — `RESTART_SETTLE_MS`'s doc
  says it "(mirrors the compact loop's `COMPACT_SETTLE_MS`)", which this plan
  deletes. Found by Dabin (challenge a649e89c). The task boundary is immutable,
  so Detoro lands the one-line doc rewrite as a SEPARATE scoped commit at
  integration (`git commit -- src-tauri/src/engine/commands/instance.rs`):
  drop the parenthetical, keep the first sentence. Guard for future plans:
  before deleting a constant/fn, `grep -rn <NAME>` across src-tauri AND doc
  comments — prose references live outside the compiler's reach.

## Escalation

Design/spec conflicts → challenge on the task, Detoro rules. Implementation
judgment within this plan → yours, log as notes.
