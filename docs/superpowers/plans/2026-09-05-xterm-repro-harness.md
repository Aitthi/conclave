# Lane R2 — xterm-repro-harness

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Marty (0ed6b21b-8322-46c6-868c-8df84218bd30, Researcher)
umbrella: `docs/superpowers/plans/2026-09-05-xterm-vscode-parity.md` (read first)

## Reading order

1. Umbrella plan — goal, established facts, constraints, risk ledger.
2. `docs/superpowers/plans/assets/2026-09-05-xterm-autocomplete-garble.png` (Read it).
3. `scripts/pty-inject-repro.py` (whole file — proven pattern for spawning `claude` in
   a PTY, driving the trust dialog, scrubbing env, TIOCSWINSZ).
4. `src/components/Terminal.tsx` L80-130 (the exact xterm options the replay must mirror)
   and `src-tauri/src/engine/runtime/pty.rs` L60-90 (the env the child gets).

## Deliverables

### 1. `scripts/pty-record.py`

Spawn a CLI in a PTY and record EVERYTHING the child writes, with timing.

```
python3 scripts/pty-record.py --bin <path-to-claude> --cwd <scratch> \
    --cols 153 --rows 55 --env TERM=xterm-256color --env COLORTERM=truecolor \
    [--env TERM_PROGRAM=vscode --env TERM_PROGRAM_VERSION=1.105.0] \
    --keys '/s at' --key-delay-ms 150 --settle 6 --out <dir>/rec.jsonl
```

- Reuses the env-scrub + trust-dialog code path from `pty-inject-repro.py`.
- `--keys` is typed one character at a time with `--key-delay-ms` between (a human
  typing `/s at`; the popup changes on every keystroke — that is the scenario).
- Output: JSONL, one record per PTY read: `{"t_ms": <since start>, "kind": "out",
  "b64": <raw bytes>}`; resize/keystroke events as `{"kind":"resize","cols":..,"rows":..}`
  / `{"kind":"key","b64":..}` so the replay can interleave them in order.
- Also writes `<out>.txt` — the raw bytes concatenated (no timing) for quick `cat -v`.
- Optional `--resize-after-ms N --resize-to CxR` to record a live resize mid-session
  (needed later for reflow experiments; implement, one flag, no more).

### 2. `scripts/xterm-replay.mjs`

Replay a recording through `@xterm/headless` and print the final screen.

```
node scripts/xterm-replay.mjs <rec.jsonl> [--convert-eol true|false] [--unicode 6|11] [--dump-every-key]
```

- Add `@xterm/headless` as a devDependency pinned to the SAME beta as our `@xterm/xterm`
  (6.1.0-beta.287; if that exact headless beta does not exist, use the nearest lower
  and note it). `@xterm/addon-unicode11` is already a dependency.
- Options MUST mirror `Terminal.tsx`: `convertEol`, `scrollback: 12000`,
  `allowProposedApi: true`, Unicode 11 active. Renderer options (`rescaleOverlappingGlyphs`,
  WebGL) have no headless equivalent — document that in the findings.
- Apply `resize` records via `term.resize(cols, rows)` at their timestamp order; feed
  `out` records via `term.write` (await the write callback before the next record so
  ordering is deterministic).
- Print: the normal buffer's last `rows` lines, right-trimmed, with a `|` gutter and
  row numbers, then the cursor position. `--dump-every-key` prints a frame after each
  `key` record so a corruption can be pinned to the keystroke that caused it.
- Exit 0; exit 2 if any line of the autocomplete block violates the layout invariant
  "a row that contains `/` followed by letters and a two-space gap starts with two
  spaces" (heuristic — print the offending rows; this is a repro signal, not a
  spec).

### 3. `docs/superpowers/specs/2026-09-05-xterm-repro-findings.md`

Run the matrix below and record, per cell, whether the screenshot corruption
reproduces (paste the offending screen rows, ≤ 12 lines each), plus the recording path.

| # | size | env | convertEol | unicode |
|---|---|---|---|---|
| A | 153×55 | ours (no TERM_PROGRAM) | true (ours) | 11 |
| B | 153×55 | ours | **false** | 11 |
| C | 153×55 | + `TERM_PROGRAM=vscode`, `TERM_PROGRAM_VERSION=1.105.0` | true | 11 |
| D | 80×24 | ours | true | 11 |
| E | 60×30 (narrow — the screenshot looks narrow) | ours | true | 11 |
| F | same recording as A | ours | true | **6** |

A, C, D, E need their own recordings (env/size differ); B and F REPLAY A's recording
with different xterm options — that is the point of separating record from replay.

Then the verdict section: (1) does headless xterm show the corruption at all? (2) which
single knob flips it? (3) if headless is clean everywhere, say so plainly — the defect
then lives in the renderer or in the live data path, and name which of the two the
evidence points to. Also `cat -v` the bytes of one corrupted frame (A, last keystroke)
and annotate the control sequences Claude Code uses for the popup (CUP? CUU+EL? `\n`
without `\r`? DECSET 2026?) — ≤ 40 lines, this is the piece lane F will read most.

## Boundary

`scripts/pty-record.py`, `scripts/xterm-replay.mjs`, `package.json`, `pnpm-lock.yaml`,
`docs/superpowers/specs/2026-09-05-xterm-repro-findings.md`. Recordings go under
`/private/tmp/claude-501/xterm-repro/` (not the repo); the findings doc names the paths.

## Gates (record each with `conclave task gate <ws> xterm-repro-harness -- <cmd>`)

- `node scripts/xterm-replay.mjs <recA.jsonl>` (exit code IS the repro signal — record
  it either way)
- `node scripts/xterm-replay.mjs <recA.jsonl> --convert-eol false`
- `pnpm tsc --noEmit` if you touched any TS (you should not need to)
- `python3 -m py_compile scripts/pty-record.py`

READY note must name: findings path, the verdict in one sentence, the knob (if any) that
flips the repro, and the recording paths.
