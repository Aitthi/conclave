# Agent chat UI — the Context drawer's MESSAGES section becomes a readable conversation

**Goal:** replace the terse direction-only rows (`to Dew · 8m`) in the Context drawer's
MESSAGES section with a chat-style timeline the human can actually read: full message
content, who said what, easy scanning. Human-picked layout: ONE merged timeline (all peers
interleaved chronologically) with per-peer filter chips — not per-peer threads.

**Owner/lead:** Detoro `bfb737ff`. UI judgment (spacing, type scale, exact colors) is the
implementer's — this plan fixes structure and content, not pixels. Progress in
`progress:agent-chat-ui`.

## Facts already verified by the lead

- `inter_agent_message` rows carry full `text`, `from_instance_id`, `to_instance_id`,
  `status`, `auto_submitted`, `created_at` (`repo/inter_agent_message.rs:39-47`).
- `ContextDrawer.tsx` already fetches them (`messages` state, ~line 109; rendering ~line
  950). VERIFY the IPC payload includes `text` end-to-end before building; if the list
  endpoint strips it, extending the payload is in scope (additive field only).

## Structure (fixed)

- Merged timeline, oldest→newest top→bottom, auto-scroll to newest on open/new message.
- Bubbles: outgoing (this agent → peer) right-aligned; incoming left-aligned with the
  peer's display name + agent color accent (roster/def `color`). Relative timestamps.
- Filter chips above the timeline: `All` + one per peer that appears in the loaded
  messages. Chip shows peer name; selecting narrows the timeline.
- Long messages clamp (~6 lines) with an expand affordance; full text on expand.
- `status` / `auto_submitted` surfaced subtly (e.g. a small "queued"/"injected" hint),
  never as loud as the content.
- Keep the existing "Accepts from" / "Auto-submit on inject" controls — compact them into
  a small header row or overflow popover; they must not push the conversation below the fold.
- Empty state and the drawer's narrow width respected: the section keeps its own
  max-height + internal scroll; no horizontal overflow.
- All UI copy English.

## Out of scope

Composing/sending messages from the drawer (read-only view for now); engine changes beyond
the additive payload field if needed; per-peer thread view (explicitly rejected by the human
for v1 — drawer is too narrow to lose the big picture).

## Gate

`npx tsc --noEmit` clean, `npm run build` clean, `cargo test --lib` untouched-green if any
Rust payload change, plus a screenshot of the new section in `progress:agent-chat-ui`.
Review by Mellow before the lead integrates.
