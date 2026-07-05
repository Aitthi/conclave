# Plan: supervisor-picker-remove-affordance

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
implementer: Dew (40d90aed-bdfe-4dfb-837b-1daa22d796b1)
reviewer: assigned at review time (Mellow or Armin, availability-weighted)
design escalation: Arta (688719b6-741d-43e1-bc6c-9a2e78d4e21b)

## Why (human feedback, 2026-07-05 ~20:4x)

The human exercised the shipped roster-chip "Change supervisor" modal (r14) and
could not find how to REMOVE an existing supervisor: "ตอนสร้างมัน skip ได้แบบ
ไม่ต้องมี supervisor ก็ได้ แต่ตอน Change มันเอาออกไม่ได้ มันบังคับเลือก".

Diagnosis (Detoro, verified by mounting SupervisorPicker standalone in vite +
Playwright click-through): the mechanics are NOT broken — the "Reports to the
human" row selects on click and Confirm delivers `onPick(null)`; the engine
Clear path (`PositionField::Clear`) is unit-tested. The defect is pure
DISCOVERABILITY: the Human row does not read as "remove supervisor". The
affordance failed its primary user; canon @3fd0f6e's edit-footer composition
(Cancel + Confirm only) is amended by this lane.

Human approved the direction verbatim: "ตามนั้นเลย พอ Arta pin เสร็จก็ให้ Dew
ทำต่อได้เลย".

## Design canon

Arta's amended proto: .arta/proto/screens/supervisor-picker.tsx @ <PIN-SHA>
(fill from Arta's reply before task creation — the task's --canon carries it).
Arta rules the exact interaction; this plan binds to that ruling:

- An explicit "Remove supervisor" secondary action in the EDIT variant only,
  rendered ONLY when the subject currently HAS a supervisor
  (`current != null`).
- Interaction (one-click write vs pre-selecting the Human row), placement, and
  whether the Human row stays are Arta's ruling — copy them from the pinned
  proto verbatim, do not improvise (drift found at the design gate was created
  here).
- Add-flow footer is OUT OF SCOPE: Skip + Add agent stays exactly as ruled in
  99ffd9ed/ddc72d4. Do not touch the add variant.

### Default spec (fires if Arta's pin misses the stated deadline)

Arta was asked 2026-07-05 ~20:51 with a 20-minute deadline and this default.
If the task's canon note names Arta's SHA, THAT wins; otherwise implement
exactly this:

- Footer, edit variant, LEFT slot next to Cancel: a text button
  `Remove supervisor`, same ghost styling class as Cancel
  (`text-[12.5px] font-medium text-text-secondary px-3 py-1.5 rounded-lg
  hover:bg-overlay/[0.05] disabled:opacity-50`).
- Rendered only when `variant === "edit" && current != null`.
- `onClick={() => onPick(null)}` — one-click write, same path Skip uses in the
  add flow (precedent: the human overruled us to keep Skip as an explicit
  one-click bypass; same mental model here).
- `disabled={submitting}`.
- The Human row STAYS unchanged (it also covers the "already reports to the
  human" state where the button is hidden).
- Misclick-trap note: Cancel and Remove supervisor sit adjacent with the same
  ghost style but different effects — mirror the 99ffd9ed remedy by keeping
  order `Cancel · Remove supervisor` (destructive-ish action farther from the
  edge) and let Arta adjust at the design gate if needed.

## Change surface

- `src/components/SupervisorPicker.tsx` — the only expected file. The
  component already receives `current`, `variant`, `onPick`, `submitting`;
  the remove affordance composes from those. No IPC/type changes.
- `src/components/Roster.tsx` — only if Arta's pinned proto requires caller
  changes (e.g. new prop). If you believe it does, file a task challenge
  BEFORE editing; the boundary lists it defensively but the null hypothesis
  is zero caller changes.

## Steps

1. Read Arta's pinned proto at the canon SHA. Extract the exact markup/copy
   for the remove affordance.
2. Implement in SupervisorPicker.tsx per proto. Wire the action to the
   existing `onPick(null)` path (one-click) or `setDraft(null)` (select) —
   whichever the proto specifies. Respect `submitting` (disable while a write
   is in flight).
3. Gate: `conclave task gate <ws> supervisor-picker-remove-affordance -- sh -c "pnpm build"`
   (commit first — the gate pins HEAD at run time).
4. READY note; Arta design pass + reviewer functional pass at review.

## Risk ledger

- The edit footer already has Cancel (left) + Confirm (right, ml-auto). A
  third action must not recreate the misclick-trap class of defect ruled in
  99ffd9ed (adjacent same-styled buttons with opposite effects) — Arta's
  proto placement is the authority; flag to Arta if the pin looks adjacent
  to Cancel with identical styling.
- `submitting` guard: the new action triggers a real write in the one-click
  variant; it must disable while submitting or a double-click double-writes.
- D6 failure UX (error banner without closing the modal) must keep working —
  the remove action goes through the same `handleSetSupervisor` caller path;
  no new error plumbing.

## Global constraints (inherited by every step)

- Shared checkout: use `conclave stage commit` scoped to the boundary; never
  raw `git add`/`git commit` (sweeps peers' staged work).
- App UI copy is English.
- Design canon is exact: markup/copy from the pinned proto, no restyle.
