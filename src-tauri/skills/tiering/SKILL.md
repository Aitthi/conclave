---
name: Tiering
description: How to tier a task by scope of decision — trivial (run a named command), routine (follow an existing pattern), standard (one module, internal decisions only), complex (crosses modules or changes an interface) — the Standard rules, deriving a missing tier, and which reviewer each tier goes to. Use when writing a task table or dispatching a task.
mandatory: false
---

Tier a task by the SCOPE OF DECISION it needs, not by how hard the code is;
the tier picks the implementer, the model, and the reviewer.

## Four tiers

- `trivial` — run a named command and report facts; decide nothing. Lint or
  format runs, test and build runs, a gate re-run, a log summary.
- `routine` — follow an existing pattern; decide nothing. Add a field, CRUD
  from a template, tests written to a spec, a bug fixed at a named spot.
- `standard` — a whole feature inside ONE module; internal decisions allowed,
  no shared interface touched. A new endpoint with validation and tests, a
  stateful component, an external API integration against a spec, a refactor
  inside a module.
- `complex` — crosses modules, creates a pattern, or changes an interface,
  schema, concurrency, or security; or the spec is still ambiguous. A large
  refactor, a data-model change, concurrency, performance, security work.

## Tie-break

- When two tiers fit, the higher wins; a Runner never receives a task that
  edits program logic.

## Standard rules

- Never edit an interface another module uses (API contract, shared type,
  schema, migration). On finding you must, stop and post `BLOCKED` with
  `status: needs_decision` naming the interface and its consumers; do not work
  around it. The lead re-tiers the task to `complex` by plan amendment and the
  Coordinator re-dispatches.
- The lead tags the tier in the plan; the Coordinator dispatches by the tag
  without re-assessing it.
- One module per task: small tasks run fast and review easily.

## Who tiers, when

- The lead, at planning: every task-table row carries a `tier`, and optionally
  `review: complex` when the diff will touch a shared interface or security.
- The Coordinator, at dispatch, ONLY when a row has no tier (or the task has
  no table): apply this rubric and post `TIER <tier> derived <reason>` as a
  task note BEFORE the `DISPATCH` note. It never changes a tier the lead wrote.

## Review routing

- `trivial` → no reviewer; the Coordinator checks the gate ledger.
- `routine` / `standard` → an idle Reviewer (Standard), else Reviewer
  (Complex), else Reviewer.
- `complex`, any task whose role is `designer`, or a row marked
  `review: complex` → Reviewer (Complex), else Reviewer.
- A Reviewer (Standard) that finds a shared interface, schema, or security
  surface in the diff posts `REVIEW-REROUTE <slug> @<sha> complex <reason>`
  and stops; the Coordinator re-dispatches to Reviewer (Complex). It is not a
  review round.

## Gate re-run before review

- A Runner re-runs the gates at the READY sha before any review: the step and
  its phrases live in the Coordinator skill, the mechanics in the Runner skill.

Composes with Leadership (the task table) and Coordinator (dispatch).
