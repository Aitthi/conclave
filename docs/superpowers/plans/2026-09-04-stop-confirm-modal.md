# Always confirm Stop actions

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Require an explicit confirmation modal before stopping either a workspace or an
agent, regardless of whether the runtime is currently emitting output.

## Reproduction and fail path

On main at `43f781f`, the default fixture reliably exposes the defect:

- Clicking **Stop** on an active but non-working agent calls `stopAgent(entry)`
  directly because `src/components/Roster.tsx` gates the dialog on
  `entry.working`.
- Clicking **Stop workspace** calls `onStopWorkspace()` directly when no agent
  is currently working because the same file gates the dialog on
  `anyAgentWorking`.
- The existing `ConfirmLifecycleDialog` already implements the required modal,
  Escape/cancel handling, focus return, and confirmed dispatch. The defect is
  the two conditional bypasses, not a missing backend or dialog component.

The falsifier is simple: if either Stop control can invoke `instance.stop` or
`workspace.stop` without first opening `ConfirmLifecycleDialog`, the fix is not
complete. Start and Resume are intentionally immediate.

## Product ruling

Confirmation is unconditional for Stop workspace and Stop agent. Runtime
`working` is transient and must not decide whether a destructive runtime action
asks for confirmation. Keep Remove agent's existing confirmation unchanged.

Use direct, state-independent copy:

- `Stop <workspace>?`
- `Stop <agent>?`

The supporting text should retain the current preservation/termination contract
without implying that confirmation appears only while work is streaming.

## File boundary

- `src/components/Roster.tsx`

## Implementation

1. Route every started-workspace Stop click through
   `openDialog({ kind: "workspace-stop" }, trigger)`.
2. Route every active-agent Stop click through
   `openDialog({ kind: "agent-stop", entry }, trigger)`.
3. Remove dead `anyAgentWorking` derivation if it has no remaining consumer.
4. Update modal titles as ruled above. Do not change Start, Resume, Remove,
   backend lifecycle semantics, or fixture data.

## Verification

- `pnpm build`
- `git diff --check`
- With the default fixture, verify an idle/non-working agent Stop opens the
  agent confirmation and Cancel causes no state change.
- Verify Stop workspace opens the workspace confirmation even in a fixture state
  with no working agent; Confirm is the only path that performs the stop.
- Run and record the standing UI gate:
  `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 stop-confirm-modal-v2 -- pnpm uishot home`
- Open and inspect `.shots/home-default.png`; attach its path in the READY note.
- Also capture and visually inspect the opened agent and workspace confirmation
  states if the available browser tooling supports interaction; attach those shot
  paths in the READY note.

## Done

Both Stop controls always open the correct confirmation modal, confirmed actions
still reach the existing lifecycle handlers, Cancel/Escape leave state unchanged,
the build is green, and the home pixel gate has been viewed and recorded.
