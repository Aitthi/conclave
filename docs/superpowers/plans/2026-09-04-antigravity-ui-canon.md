# Antigravity CLI and provider-label UI canon

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Create the production UI canon for configuring an Antigravity CLI agent and identifying its provider in dense roster/supervisor surfaces.

## Ground truth

- Product register: quiet, precise, operational; dense macOS-like controls, restrained accent use, no invented settings widgets.
- Existing Builder and `src/styles/app.css` are visual/system canon.
- Provider placement from human-approved task `roster-provider-badge`: provider/model chip sits on the agent name line and replaces the generic terminal glyph; SupervisorPicker also carries the same shared chip.
- Antigravity UI data contract: `cliKind="antigravity"`, provider label `Antigravity`, optional free-text model, optional effort `Auto|low|medium|high`, execution mode `Default|Accept edits|Plan|Bypass permissions`.
- Do not infer Google/Anthropic/OpenAI from arbitrary model strings. When a model exists display `Antigravity · <short model>`; otherwise `Antigravity`.
- `rtk` and AGY sandbox controls are not offered in v1. Switching from Claude/Codex `auto` permission mode normalizes to AGY Default, never bypass.

## Deliverable

Create `design/screens/antigravity-cli.tsx` showing:

- Builder with Antigravity as an enabled CLI segment.
- Model field with `Auto`/blank behavior and an example named model; never hardcode the authenticated model list.
- Effort control and Execution mode with concise, accurate help text; dangerous bypass has the only warning treatment.
- Missing-`agy` validation/error state with a useful install direction, without storing an absolute executable path.
- Roster row and Change supervisor candidate using the shared provider/model chip; long model truncation plus full accessible/title text; light and dark treatment.
- Keyboard/focus, disabled, validation and responsive states consistent with existing components.

Use fixed data. Do not edit `src/` product files. Attach READY with canon SHA, visual QA image paths, affected real-app view IDs and exact pixel acceptance criteria.
