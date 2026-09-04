# Agent stop/resume UI canon

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Create the canonical roster/home visual and interaction specification for starting/stopping a workspace and stopping/resuming an individual workspace agent without removing either.

## Deliverable

Create `design/screens/agent-stop-resume.tsx` as the design canon grounded in the current `src/components/Roster.tsx` and `src/components/WorkspacePane.tsx` UI. It must show or document:

- Stopped workspace on entry: the user can inspect retained workspace information without launching any agent, and the center pane offers an explicit `Start workspace` action.
- Started workspace: a clear `Stop workspace` control; stopping immediately tears down every live agent runtime while retaining workspace, agents, configuration, tasks, messages, and history.
- Starting a workspace launches only agents whose individual availability is active; individually stopped agents stay stopped.
- Workspace start/stop loading, partial-launch failure, retry, and confirmation behavior. Stopping a workspace with working agents requires explicit confirmation.
- Active idle agent: Stop action remains distinguishable from destructive Remove.
- Active working agent: Stop is immediate after an explicit confirmation that the current runtime/work is terminated; membership, configuration and history are retained.
- Stopped agent: persistent labelled `Stopped` state, Resume primary lifecycle action, Remove still available and destructive.
- Selected stopped agent: a useful center-pane stopped state with a Resume action instead of an infinite/opening-session state.
- Routing: stopped agents are visibly unavailable and cannot be selected as message recipients.
- Loading, success and error behavior for Stop and Resume; keyboard/accessibility labels and focus behavior.
- Copy must avoid confusing agent lifecycle Resume with snapshot/context resume.

## Constraints

- No product source edits.
- Preserve the established visual language and information density; this is an additive lifecycle control, not a roster redesign.
- Use fixed example data only.
- Note affected real-app view IDs and both default/empty screenshot expectations for the implementation gate.

## Done

Attach a READY note with the canon path and the exact UI acceptance checklist for a zero-context implementer.
