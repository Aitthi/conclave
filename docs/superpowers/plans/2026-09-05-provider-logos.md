# Provider logo marks for the Runtime picker

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Goal

Deliver small SVG logo marks for the CLI runtimes the agent Builder's
Runtime picker shows (spec D5, `docs/superpowers/specs/2026-09-05-agent-builder-redesign-design.md`),
with their license recorded, so the designer and implementer can use them
without a second sourcing round.

## Runtimes

| cliKind | Display name | Product |
|---|---|---|
| `claude-code` | Claude Code | Anthropic Claude Code |
| `codex` | Codex | OpenAI Codex CLI |
| `antigravity` | Antigravity | Google Antigravity (`agy` CLI) |
| `opencode` | opencode | https://opencode.ai |
| `muse-spark` | Muse Spark | Muse Spark (identify the vendor and canonical mark; if ambiguous, record the candidates and pick the one the human's screenshot shows as "Muse") |

## Primary source

https://github.com/Untrivial-ai/agent-orchestrator — its README/provider
table renders exactly these marks (the human's screenshot shows Claude Code,
Codex, opencode, Muse, Agy among others). Locate the icon files in that repo
(likely under a `public/`, `assets/`, or `icons/` directory, or inline in a
TSX map), and read the repository LICENSE.

## Deliverable

1. `design/assets/providers/<cliKind>.svg` — one file per runtime, viewBox
   normalised to `0 0 16 16` or `0 0 24 24`, fills converted to
   `currentColor` (keep a second `<cliKind>.color.svg` if the original is
   multi-colour and worth keeping).
2. `docs/research/2026-09-05-provider-logos.md` — for each mark: source URL,
   commit SHA it was taken from, license of the source repo, and whether the
   mark is the vendor's official brand asset or a community redraw. Flag any
   mark whose license or trademark terms would block bundling it in a
   shipped app, with the alternative (vendor press kit URL).

## Constraints

- No `src/` edits. No product code.
- Do not fetch from sites that require login. Use `conclave browser` for any
  page that needs rendering; `curl`/`gh api` for raw files.
- Fixed facts only; if a mark cannot be found, say so and leave the file
  absent rather than drawing one.

## Gate

`conclave task gate <ws> provider-logos -- ls design/assets/providers` and
`conclave task gate <ws> provider-logos -- pnpm build` (tsc must not pick up
anything new).

## Done

READY note: list of delivered files, the commit SHA (commit via
`conclave stage commit` on main), and the license summary line. Escalation:
Detoro (30fa04f4).
