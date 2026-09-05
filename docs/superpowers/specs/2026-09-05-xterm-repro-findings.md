# xterm autocomplete reproduction findings

## Verdict

The reported autocomplete-row corruption does **not** reproduce in
`@xterm/headless` `6.1.0-beta.287` in any of the six matrix cells, including the
autocomplete frames produced after `/` and `/s`. No tested knob flips that repro:
the complete per-key screen dumps for `convertEol:true` and `convertEol:false` are
byte-for-byte identical after normalizing the option label, and the dumps with
`TERM_PROGRAM` unset versus `TERM_PROGRAM=vscode` are identical after normalizing
the scratch path.

This evidence rules out `convertEol` as the mechanism for this recording: Claude
emitted zero bare LF bytes in every typed-key slice, so xterm never had an LF on
which `convertEol` could act. The clean headless buffer points more strongly to a
browser renderer defect (stale cells) than to the parser/buffer. It does not rule
out Conclave's live Rust/Tauri output path dropping or reordering data, because the
recorder reads the PTY directly and bypasses that transport.

Unicode 6 produces a separate, visible status-line artifact (`⚡  High     )`)
where Unicode 11 renders `⚡ High`; that is not the reported autocomplete defect
and is evidence to keep Unicode 11 active.

## Environment and harness

- Claude Code: `2.1.261`
- Node: `v22.23.1`; Python: `3.14.2`
- `@xterm/xterm`: `6.1.0-beta.287`
- `@xterm/headless`: `6.1.0-beta.287` (exact match, not a nearest-lower fallback)
- `@xterm/addon-unicode11`: `0.10.0-beta.287`
- Common child env: `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `LANG=en_US.UTF-8`; inherited `CLAUDE_CODE*` and `CLAUDECODE` variables scrubbed.
  `TERM_PROGRAM*` is also scrubbed by default and is present only when supplied
  explicitly with `--env`, making the A/C distinction independent of the parent shell.
- Review follow-up (credit: Mellow): recordings A–E were made while the recorder
  still re-injected `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1` after that scrub.
  `pty-record.py` no longer sets it, matching production `pty.rs`; the recordings
  were not repeated because this transcript-persistence flag has no rendering effect.
- Input: `/s at`, one character every 150 ms, followed by a 6-second settle.

`scripts/pty-record.py` records each PTY read as base64 plus ordered key and resize
events. `scripts/xterm-replay.mjs` awaits every xterm write callback and checks the
layout after every key-delimited frame, not only the final screen. This matters in
Claude Code 2.1.261: the autocomplete is visible after `/` and `/s`, but closes
after the later characters, so a final-screen-only assertion would be vacuous.
The replay exits 0 only when it observed at least one clean autocomplete frame,
2 for a layout violation, and 3 when no autocomplete block was detected in any frame.

The replay mirrors the application options that exist in headless xterm:
`convertEol`, `scrollback: 12000`, `allowProposedApi: true`, and Unicode 11. The
headless package has no WebGL/DOM renderer, font metrics, or
`rescaleOverlappingGlyphs` behavior; those cannot be evaluated here.

## Six-cell matrix

Every `PASS` below means all key-delimited autocomplete frames satisfied the
invariant and the replay exited 0. There are no offending rows to paste.

| Cell | Size / environment | Replay option delta | Recording | Result |
|---|---|---|---|---|
| A | 153×55, no `TERM_PROGRAM` | `convertEol:true`, Unicode 11 | `/private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-a.jsonl` | PASS; no corruption |
| B | A recording | `convertEol:false`, Unicode 11 | same as A | PASS; every per-key screen equals A |
| C | 153×55, `TERM_PROGRAM=vscode`, `TERM_PROGRAM_VERSION=1.105.0` | `convertEol:true`, Unicode 11 | `/private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-c.jsonl` | PASS; every per-key screen equals A after cwd normalization |
| D | 80×24, no `TERM_PROGRAM` | `convertEol:true`, Unicode 11 | `/private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-d.jsonl` | PASS; no corruption |
| E | 60×30, no `TERM_PROGRAM` | `convertEol:true`, Unicode 11 | `/private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-e.jsonl` | PASS; no corruption |
| F | A recording | `convertEol:true`, Unicode 6 | same as A | PASS for autocomplete; unrelated stray `)` on the status line |

Recording SHA-256 values:

```text
d383defee9afe7462cd513ff752ec2694c4626783c8d4b1e327394d600245fd7  rec-a.jsonl
e66f13722da9f26a7351c37549ccbc2cc1bbbe9896d41e8d6f19803ad0958cc0  rec-c.jsonl
1f0d47fba5e9548a0d3c8a6ad633fa5d2da7e4c8bc83ae2fca41ca08548a8200  rec-d.jsonl
0052339f6f3c8ea52b3fb201ec55555d7a3d146ec7887cf4ed8b2292b9e44471  rec-e.jsonl
```

Representative clean A frame after `/s`:

```text
   8 |❯ /s
  10 |  /status                 Show Claude Code status including version, model, account, API connectivity, and tool statuses
  11 |  /skills                 List available skills
  12 |  /subtask                Send a subagent off with your full context; its result comes back here
  20 |  /statusline             Set up Claude Code's status line UI
  25 |  /setup-matt-pocock-skills  Configure this repo for the engineering skills…
```

## Raw repaint bytes and control sequences

There was no corrupted A frame to excerpt. The following is the requested
`cat -v` output for A after the last key (`t`), the slice that leaves the clean
final screen:

```text
^[[?25l^[[6D^[[4B^M^[[6C^[[4At^M^M
^M
^M
^M
^[[7C^[[4A^[[?25h
```

The exact 47-byte slice is:

```python
b'\x1b[?25l\x1b[6D\x1b[4B\r\x1b[6C\x1b[4At\r\r\n\r\n\r\n\r\n\x1b[7C\x1b[4A\x1b[?25h'
```

Annotation:

- `CSI ?25l` / `CSI ?25h`: hide/show the cursor around the repaint.
- `CSI 6D`, `CSI 4B`, `CR`, `CSI 6C`, `CSI 4A`: relative cursor movement to the
  input position; then Claude prints `t`.
- Four row moves are `CR LF` pairs. There are 6 CR, 4 LF, and **0 bare LF** in
  this slice.
- `CSI 7C`, `CSI 4A`: restore the cursor after the row moves.
- No CUP (`CSI … H`), no DECSET 2026 synchronized-output on/off, and no EL occur
  in this last-key slice.

Across the autocomplete-producing frames, Claude primarily uses CHA (`CSI … G`),
relative CUU/CUD/CUF/CUB, CRLF, and EL (`CSI K`). Clearing the popup after `a`
uses 26 EL, 25 erase-entire-line (`CSI 2K`), and 25 CUU (`CSI 1A`) operations.
No typed-key slice contains a bare LF or DECSET 2026. Therefore toggling
`convertEol` cannot alter this recording's buffer result, which the identical
per-key dumps confirm empirically.

## Experiment ledger and harness checks

1. The first readiness attempt failed because Claude's styled trust-dialog output
   concatenates as `trustthisfolder`; changing the detector from `\s+` to `\s*`
   made the prompt detectable.
2. The second attempt showed that the trust dialog's incremental redraw moves only
   the cursor and does not re-emit the newly selected `Yes` label. The recorder now
   drives the deterministic initial `No` selection with one Down+Enter sequence.
3. A–F then ran successfully and produced the results above.
4. A synthetic malformed autocomplete block (`broken-row  desc` between two valid
   command rows) makes `xterm-replay.mjs` exit 2 and print the offending row.
5. A live-resize self-check recorded 80×24 followed by 100×30 at +100 ms and
   replayed at 100×30 with the invariant passing:
   `/private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-resize-final.jsonl`.

## Implication for the fix lane

Do not remove `convertEol` as a fix for this screenshot based on the current
evidence; the proposed mechanism requires a bare LF and Claude emits none in the
relevant frames. `TERM_PROGRAM=vscode` may still be desirable for broader VS Code
parity, but it does not change this reproduction. Keep Unicode 11.

The next discriminating test should replay/capture the same interaction in the real
browser renderer with GPU enabled while also preserving the exact chunks delivered
by `session:output`. A clean delivered-byte log plus corrupted pixels would isolate
the renderer; a divergent delivered-byte log would isolate the Rust/Tauri live data
path.
