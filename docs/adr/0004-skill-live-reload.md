---
status: accepted
---

# Skill live reload: the sidecar file is the mutable source of truth, updated in place and nudged

ADR 0001 delivered skill content via a per-instance sidecar file written once at launch, with a
"Restart to apply" badge when attachments drift. Two field problems with that design:

1. Agents lost their skills after `/clear` — the sidecar's content reaches the model only through
   a file read whose result lives in conversation history, which `/clear` erases. (Fixed first at
   the prompt layer: the pointer sentence and the compact/resume restore prompts now order a
   re-read on every fresh context.)
2. Changing a live agent's skills required a full restart, destroying its context to deliver a
   text change.

Decision: make the sidecar the LIVE source of truth for the whole instance lifetime.

- **The pointer is unconditional.** Every `cli` launch writes the sidecar and appends the pointer
  sentence — even when no skills are attached (the file then holds a one-line placeholder). The
  alternative (skip when empty, as v1 did) permanently cuts off an agent launched skill-less from
  any later attachment. Rejected.
- **Skill mutations rewrite the sidecar in place and nudge live instances.** `agent.save`
  (attach/detach/builtin selection) and `skill.save`/`skill.delete` (content edits) rewrite the
  sidecar of every affected instance and inject a single-line "your standing instructions were
  updated — re-read the file" prompt into each live one. The alternative — rewrite silently and
  wait for the next fresh context — leaves long-running agents on stale skills indefinitely.
  Rejected. The nudge names the sidecar's full path so it works even for instances launched
  before this change (whose preamble has no pointer).
- **The sidecar keeps full concatenated content, not an index of per-skill files.** One read
  gets everything; an index risks the agent reading the list and skipping the bodies, and costs
  N+1 reads per fresh context. Rejected.
- **`session.launched_skill_ids` is refreshed on live reload**, so the staleness badge (ADR 0001)
  clears itself: after a successful rewrite + nudge the running instance IS current. Restart
  stops being the delivery mechanism for skills.
