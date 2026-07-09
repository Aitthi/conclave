# Product

## Register

product

## Users

Conclave is used by engineers and technical leads who coordinate several AI CLI
or chat agents inside a project workspace. They are usually in a focused work
session, switching between roster, chat, task board, memory, artifacts, and agent
configuration while live agents continue running.

## Product Purpose

Conclave is a native macOS app for orchestrating multi-agent software work. It
turns reusable `AgentDefinition`s into workspace-scoped `WorkspaceAgent`s,
launches Claude Code or Codex with the right identity and skills, and keeps
coordination state such as tasks, blackboard notes, memory, context meters, and
inter-agent messages visible enough for a lead to make decisions.

Success means the operator can see who is doing what, trust each agent's state,
configure launches without leaving the workflow, and recover the reasoning behind
decisions from durable records instead of chat memory.

## Brand Personality

Quiet, precise, operational. The interface should feel like a serious native
tool for repeated use: dense enough for supervision, restrained enough to stay
readable, and explicit where agent autonomy could otherwise become ambiguous.

## Anti-references

Do not make Conclave feel like a marketing SaaS dashboard, a decorative AI
workflow canvas, or a novelty chat app. Avoid oversized hero language,
ornamental gradients, invented controls for standard settings, and visual
effects that compete with the user's task.

## Design Principles

1. Keep state attributable: every live status, context meter, task event, and
   message should make its owner clear.
2. Prefer durable records over transient explanation: plans, gates, roles, and
   launch settings should be inspectable without reading chat history.
3. Match density to supervision: panels can be compact, but labels, values, and
   error states must remain scannable.
4. Preserve native product familiarity: use established macOS-like surfaces,
   restrained accent use, standard form controls, and consistent icon language.
5. Make autonomy configurable, not mysterious: launch flags, permissions, model
   choices, and context limits should map visibly to what the underlying CLI
   receives.

## Accessibility & Inclusion

Use product UI defaults that meet WCAG AA contrast for text and controls. Motion
should communicate state changes and respect reduced-motion preferences. Color
must not be the only indicator for status, role, or destructive/permission-heavy
actions.
