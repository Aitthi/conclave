# Plan: Terminal tab mode — remount + serialized context restore (with keep-alive fallback)

Date: 2026-07-04 · Owner/lead: Detoro (bfb737ff) · Authority: in-loop (human-requested feature)
Implementer lane: `lane/term-remount` (Tiësto) · Reviewer: Mellow (LAND gate)
bb keys: `plan:term-remount` · `claim:term-remount` · `progress:term-remount`

## Goal (human's words, translated)

Stop keeping inactive tab terminals mounted-but-hidden. On tab switch, unmount and
remount xterm.js fresh — but restore the pre-remount context above, separated by a
clear divider line, with the live TUI continuing below. AND keep the old behavior
available behind an option, so if the new mode turns out bad we can revert cleanly.

## Decision summary (rulings, final)

1. **Two modes behind ONE dev-level flag — NO user-facing UI** (human's explicit
   ruling: this is not a user concern). The flag is a code constant with an
   optional localStorage override for switching in a dev build without a rebuild:
   - `"remount"` (NEW DEFAULT): only the active tab's `<Terminal>` is mounted;
     inactive tabs unmount. Context restored via serialize addon on next mount.
   - `"keep-alive"` (today's behavior, kept verbatim): all tab terminals stay
     mounted, visibility toggled by the `hidden` class.
2. **Snapshot mechanism**: `@xterm/addon-serialize` (pick the beta matching the
   `@xterm/xterm 6.1.0-beta.287` line in package.json). Serialize on unmount,
   `term.write()` back on mount, divider after.
3. **Divider**: written between restored context and live output. Dim, full width
   of the SAVED snapshot's cols (see spec), English label. UI copy is English per
   workspace convention.
4. **No Settings UI, no design canon**: nothing user-visible changes except the
   divider line inside the terminal buffer. `Settings.tsx` is untouched. All
   escalations go to Detoro.

## Files (exact)

- `package.json` — add `@xterm/addon-serialize` (compatible beta).
- `src/lib/termMode.ts` — NEW, small:
  `export type TermTabMode = "remount" | "keep-alive"`;
  `const DEFAULT_MODE: TermTabMode = "remount"` (the one-line revert point —
  flipping this constant back to `"keep-alive"` restores today's behavior);
  `export function getTermTabMode(): TermTabMode` — returns the localStorage
  override `conclave.termTabMode` if set to a valid value, else DEFAULT_MODE;
  read wrapped in try/catch like theme.ts:20-23. Read ONCE at module load —
  no setter, no subscribe, no live switching (a dev flips it in the console
  via `localStorage.setItem('conclave.termTabMode', 'keep-alive')` + reload).
- `src/components/Terminal.tsx` — snapshot save/restore (spec below).
- `src/components/WorkspacePane.tsx` — mode-aware tab rendering (~lines 354-380).

## Terminal.tsx spec

> **AMENDMENT (2026-07-04, post-implementation, lead self-review):** the original
> spec below implied save/restore could run unconditionally because "nothing ever
> unmounts in keep-alive". That claim was WRONG (lead's plan defect, found by lead
> during pre-LAND diff audit): `WorkspacePane` is keyed by
> `` `${workspaceId}:${agentsVersion}` `` (AppShell.tsx:288) and the LaneBoard
> branch swaps the pane out entirely — so workspace switches, agent add/remove,
> and LaneBoard open/close ALL unmount terminals even in keep-alive mode. An
> ungated restore would therefore inject snapshot+divider into keep-alive,
> breaking the human's explicit verbatim-revert requirement. RULING: gate BOTH
> the restore block and the cleanup save on `getTermTabMode() === "remount"`.
> Keep-alive must remain byte-for-byte today's behavior (buffer lost on those
> unmounts, repainted by the jiggle) — that loss IS the old behavior being
> reverted to. Side effect worth noting: in remount mode, context now also
> survives workspace switches and LaneBoard visits — a strict improvement there.

- Module-level store OUTSIDE the component (survives remounts, dies on reload —
  same lifetime as today's hidden-tab approach, no regression):
  `const snapshots = new Map<string, { data: string; cols: number }>()` keyed by
  sessionId.
- Load `SerializeAddon` unconditionally at terminal creation (cheap; needed for
  save even when the user later flips modes).
- **Restore (mount)**: synchronously after `term.open(el)` and BEFORE
  `termRef.current = term` is set (so no live chunk can interleave):
  if `snapshots.get(sessionId)` exists, write in this order:
  1. `snap.data`
  2. `"\x1b[0m\r\n"` (explicit SGR reset — serialize output is not guaranteed to
     end reset)
  3. dim divider: `"\x1b[2m" + line + "\x1b[0m\r\n"` where `line` is
     `"─── earlier output ───"` centered/padded with `"─"` to `snap.cols` chars
     (the saved width matches the hard-wrapped snapshot content; do NOT use the
     new term.cols — fit hasn't run yet at this point, it's still 80).
  The existing mount jiggle (`Terminal.tsx:159-168`) then SIGWINCHes the child,
  which repaints the live frame BELOW the divider. Do not touch the jiggle.
- **Save (cleanup)**: in the effect cleanup, BEFORE `term.dispose()`:
  `snapshots.set(sessionId, { data: serializeAddon.serialize({ scrollback: 2000, excludeAltBuffer: true, excludeModes: true }), cols: term.cols })`
  — but ONLY if at least one live output chunk was written during THIS mount
  (track with a `receivedOutputRef` boolean set inside the `useSessionOutput`
  handler, reset to false in the mount effect). Guards two clobber cases:
  (a) React 19 StrictMode dev double-mount — the first cleanup would otherwise
  overwrite a good snapshot with an empty buffer; (b) a fast tab flip before any
  output arrives.
- `excludeAltBuffer: true` is deliberate: for an alt-screen TUI the transcript
  worth restoring is the NORMAL buffer's scrollback; the live alt-screen frame is
  re-established by the jiggle's SIGWINCH repaint. Do not serialize the alt
  buffer.

## WorkspacePane.tsx spec

- Read the mode ONCE via `getTermTabMode()` (module scope or on first render —
  it cannot change within a page lifetime, so no subscription machinery).
- `remount` mode: render ONLY the active tab's Terminal subtree. `keep-alive`
  mode: current code path verbatim (`hidden` class toggle).
- Update the long mounted-tabs comment at ~354 to describe both modes.
- Do NOT remove the `getBoundingClientRect().width === 0` guard in
  Terminal.tsx:137 — keep-alive mode still depends on it.

## Risk ledger

- **Serialize addon version**: must match the xterm 6.1.0-beta line; a mismatched
  stable version will fail at load. Check npm dist-tags before picking.
- **Hard-wrap**: restored context is pre-wrapped at the old cols; it will not
  reflow on resize. Accepted (context display only). The divider at saved cols
  makes this visually coherent.
- **StrictMode double-mount** — covered by the receivedOutput guard above; verify
  in dev that no stray divider accumulates on each remount without new output.
- **Snapshot growth**: `scrollback: 2000` bounds each entry; Map is bounded by
  session count (≤ agents). No cap needed; note it in a comment.
- **Write ordering**: snapshot write happens before termRef is set — a late
  useSessionOutput event can never land mid-restore.

## Gates (implementer runs, lead reproduces post-merge)

- `npx tsc --noEmit` → 0 errors (main repo; bare worktrees lack node_modules —
  Tiësto's symlink technique).
- `npx vite build` → clean.
- Manual smoke (lead/human post-merge, app rebuild not required — `npm run dev`
  acceptable): switch tabs in remount mode → old context + divider + live frame;
  then `localStorage.setItem('conclave.termTabMode', 'keep-alive')` + reload →
  today's behavior verbatim.

## POST-LAND ledger

- **MERGED @ 23ccff0** (lane head 5dba1a4, Tiësto). One CHANGES-REQUESTED round:
  lead's pre-LAND audit found save/restore ungated by mode — root cause was this
  plan's own false claim that keep-alive never unmounts (amended above, 60a86d5);
  Mellow independently confirmed the same defect before receiving the heads-up.
  Fixed in 5dba1a4 (isRemount gate on both blocks + honest comments); Mellow
  delta re-LAND PASS 0 blocking. Gates lead-reproduced on merged main: tsc 0,
  vite build clean (pnpm install run for the new addon).
- Non-blocking note on record (Mellow): SerializeAddon loads unconditionally but
  only serializes in remount mode — acceptable since the mode is read once per
  page, no in-page flip exists to preserve a snapshot for.
- The running app (r5) predates this merge — the feature goes live at the next
  rebuild. `npm run dev` exercises it immediately.

## Out of scope

- Backend (Rust) ring buffer / reload survival — recorded as the natural follow-up
  ("ท่า B") if reload persistence is ever wanted; composes with this work.
