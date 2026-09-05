# Terminal file-drop insert travels as ONE bracketed paste

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
priority: LOW — follow-up G4 from Mellow's review of task inject-bracketed-paste (note bce384c3).
supersedes: task terminal-filedrop-paste / docs/superpowers/plans/2026-09-04-terminal-filedrop-paste.md
(its task 2, the SkillAssistPanel submit CR, shipped in Aoki's skill-assist-repair, main 0c07392 — see
`src/components/SkillAssistPanel.tsx` ~line 399).

## Context

Task inject-bracketed-paste (main 98fbbe9) made every TEXT write to a PTY agent travel as one bracketed paste
(`Runtime::send_stdin_paste`, `ipc.message.send({ paste: true })`), because macOS delivers PTY input in 1022-byte
reads and Claude Code keeps only the last un-bracketed burst chunk. The file-drop insert in `Terminal.tsx` was
outside that lane's boundary and still goes raw.

## Task (one)

`src/components/Terminal.tsx`: the `useFileDrop` callback (~line 130) calls `sendStdin(paths…join(" ") + " ")`,
which writes raw through the ordered stdin chain. Dropping several long paths (>1022 bytes) truncates exactly like
the old message path did. Fix: keep the ONE ordered chain (`stdinChainRef`, emission order is load-bearing — see the
comment above `sendStdin`) but let the drop write carry `paste: true`. Shape: `sendStdin(text, { paste })` with an
optional second arg forwarded to `ipc.message.send({ sessionId, text, paste })`; every other caller (xterm `onData`,
wheel arrows) stays byte-exact raw with no second arg. Update the drop comment: the text now arrives as one
bracketed paste, so the TUI inserts it verbatim (no submit, trailing space kept).

## Gates

`npx tsc --noEmit`; `pnpm uishot home` (open the PNG — terminal pane is empty in fixture mode, that is accepted);
`conclave task gate` for both. Manual check after the next relaunch: drop two files with long paths onto a Claude
Code pane, both paths land in the composer (recorded on bb verify:post-relaunch-2026-09-05).

## Boundary

`src/components/Terminal.tsx` only.
