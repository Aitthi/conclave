# Lane R3 — xterm-repro-h1-replay

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Marty (0ed6b21b-8322-46c6-868c-8df84218bd30, Researcher)
umbrella: `docs/superpowers/plans/2026-09-05-xterm-vscode-parity.md`

## Purpose

Confirm or kill hypothesis H1 of `docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md`
§3 in headless xterm: Claude Code writes with RELATIVE cursor moves from its own model, so a
window in which xterm's grid ≠ the PTY's grid should leave column-0 rows until the next full
repaint. Lane F (`xterm-parity-fix`) is already implementing the parity changes; this lane
produces the evidence that says WHICH change mattered, and a regression check lane F reruns.

## Reading order

1. Audit §3 H1 (the experiment is specified there) and §1 rows 9, 10, 13.
2. `scripts/xterm-replay.mjs` (yours), `docs/superpowers/specs/2026-09-05-xterm-repro-findings.md`.
3. `src/components/Terminal.tsx` L140-170 and L267-280 (the mount window being simulated).

## Deliverable

Extend `scripts/xterm-replay.mjs` with two flags, then run and document.

- `--start-size CxR` — construct the headless terminal at this size instead of the
  recording's first `resize` record (simulates the xterm 80×24 default at mount).
- `--resize-at-ms N` — apply the recording's real size (first `resize` record) at recording
  time N (simulates `fit()` after the 200 ms deferral). Records before N are written into the
  start-size grid.
- Optional (only if cheap): `--jiggle` — at `resize-at-ms` do `resize(C, R-1)` then
  `resize(C, R)`; the SIGWINCH repaint bytes are already in the recording so nothing is
  written, this only reproduces the xterm-side grid dance.

Run on the existing recordings (no new `claude` runs needed):

| # | recording | start-size | resize-at-ms | expected if H1 true |
|---|---|---|---|---|
| G | rec-a (153×55) | 80×24 | 200 | col-0 rows / offset patches until a full repaint |
| H | rec-a | 80×24 | 2000 (covers the initial paint) | worse than G |
| I | rec-a | 153×55 (control) | – | PASS, identical to cell A |
| J | rec-e (60×30) | 80×24 | 200 | offsets in the other direction (narrower→wider) |

Write a `## H1 replay (R3)` section appended to
`docs/superpowers/specs/2026-09-05-xterm-repro-findings.md`: per cell, exit code + ≤ 12
offending rows, then a verdict: does the size gap alone (G/H) produce the col-0 signature?
Note that a real Claude Code re-renders fully on SIGWINCH, so the recording's bytes are for a
child that never saw a size change — say so, and state what G/H can and cannot prove.

## Boundary

`scripts/xterm-replay.mjs`, `docs/superpowers/specs/2026-09-05-xterm-repro-findings.md`.
No `src/` edits. Lane F owns `package.json`; do not touch it (if the bumped headless from lane
F lands first, just rerun).

## Gates

- `node scripts/xterm-replay.mjs <rec-a> ` (control, exit 0)
- `node scripts/xterm-replay.mjs <rec-a> --start-size 80x24 --resize-at-ms 200` (record the
  exit code whatever it is — 2 is the H1 signal)
- `node scripts/xterm-replay.mjs <rec-e> --start-size 80x24 --resize-at-ms 200`

READY note: findings section path, one-sentence verdict, cell exit codes.
