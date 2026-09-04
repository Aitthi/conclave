# Terminal file-drop insert + skill-assist submit: finish the text-as-paste sweep

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
priority: LOW — follow-ups G4 and note (3) from Mellow's review of task inject-bracketed-paste (note bce384c3).

## Context

Task inject-bracketed-paste (main 98fbbe9) made every TEXT write to a PTY agent travel as one bracketed paste
(`Runtime::send_stdin_paste`, `message.send {paste: true}`), because macOS delivers PTY input in 1022-byte reads and
Claude Code keeps only the last un-bracketed burst chunk. Two text paths were left out of that lane's boundary.

## Tasks

1. `src/components/Terminal.tsx` — the file-drop insert (`ipc.message.send({ sessionId, text: insert })`, ~line 72)
   is TEXT: a multi-file drop over 1022 bytes truncates the same way. Pass `paste: true` on THAT write only; every
   other write in this file (xterm `onData`, wheel arrows) is a keystroke and must stay byte-exact raw.
2. `src/components/SkillAssistPanel.tsx` — `handleSend` pastes the draft but never sends the submit `\r`, so the text
   sits in the composer (pre-existing). Mirror `StdinBar.tsx`: paste, wait 40 ms, send `"\r"` raw.

## Gates

`npx tsc --noEmit`, `pnpm uishot home` and `pnpm uishot library` (open the PNGs), and — for (1) — a manual check in
the real app: drop two files with long paths onto a Claude Code pane, confirm both paths land in the composer.

## Boundary

`src/components/Terminal.tsx`, `src/components/SkillAssistPanel.tsx`.
