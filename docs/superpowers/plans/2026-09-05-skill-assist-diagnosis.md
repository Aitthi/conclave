# Skill creation failure: diagnosis
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 (Aoki) · authority: in-loop
Implementer: Dew. Escalation: Aoki. Read this plan then task brief.

## Goal and evidence
Human requests repair of nonfunctional skill creation. Screenshot /Users/detoro/Desktop/Screenshot 2569-09-05 at 11.01.39.png shows New skill, blank name/description/content, editing disabled, Ask agent to help marked running, Claude Code trust-folder prompt for app-support/skill-drafts/2d56af76-e2b6-42ba-bbdc-6a6f28ee809f. Narrow terminal has severe text wrapping.

## Scope and decisions
Diagnosis ONLY first; no product edits until Aoki records implementation ruling. Claim task (no worktree needed read-only). Trace src/components/SkillEditor.tsx, SkillAssistPanel.tsx, src-tauri/src/engine/commands/skill_draft.rs and relevant runtime launch/input code, repo skill draft watcher/save path. Read applicable repo instructions and diagnose skill. Construct deterministic feedback loop reproducing the screenshot failure; record commands, actual observed output, ranked falsifiable causes, proposed minimal repair and exact file boundary. Check fresh draft startup, initial instruction submission, subsequent user prompt, draft updates, stop/unlock/save. Do not modify user data, trust settings, kill existing sessions, or invoke paid model generation without reporting need; isolated temporary harnesses allowed.

## Coordination and risks
Existing planned terminal-filedrop-paste owns Terminal.tsx and SkillAssistPanel.tsx under Detoro. Do not edit either; Aoki coordinates overlap. No broad trust bypass. Distinguish startup trust prompt from permissions flags, raw LF vs submit CR and bracketed paste, PTY columns mismatch, frontend lifecycle failure. Report evidence before fixing. READY note with reproduction and recommended scope; notify Aoki only.

---

# Diagnosis (Dew, 2026-09-05) — reproduced

Three INDEPENDENT defects. D1 is the blocker in the screenshot; D2 and D3 each
independently prevent a successful skill draft even after D1 is fixed, so a
repair that lands only D1 will still look broken to the human.

## Reproduction harness

`scratchpad/trust-probe.py` (session-local; re-derivable from the shape below).
Spawns `claude` on a real PTY in a throwaway dir, reads output for 6s, then
SIGKILLs. It never answers the dialog and never submits a prompt, so no model
call is made and no trust setting is written — verified: `~/.claude.json` held
94 project entries before and after every run, 0 of them under the probe paths.

    pty.fork() -> chdir(dir); execvp("claude", args)
    TIOCSWINSZ cols x rows; read for N seconds; SIGKILL

**Detection trap that cost two false negatives:** Claude Code emits each WORD
followed by an absolute-column escape (`ESC[<n>G`) *instead of* a space —
`Quick` `ESC[8G` `safety`. Matching the raw bytes for "Quick safety check"
fails, and stripping CSI to the empty string yields "Quicksafetycheck" and
fails again. Substitute a SPACE for each escape before matching. My first two
probe runs reported "no dialog" for this reason alone; only dumping the raw
bytes caught it.

| Probe | cwd | flags | trust dialog |
|---|---|---|---|
| A | fresh UUID draft dir | `--permission-mode bypassPermissions` (what the app sends) | **YES** |
| B | fresh UUID draft dir | none | **YES** |
| C | `/Users/detoro/code/codeup` (already trusted) | `--permission-mode bypassPermissions` | no |
| D | fresh UUID draft dir | `--dangerously-skip-permissions` | **YES** |
| E | brand-new subdir of a trusted **git** dir | none | no |
| F | nested subdir of an **untrusted** git repo | none | **YES** (names the cwd, not the git root) |
| G | brand-new subdir of a trusted **non-git** dir | none | no |

## D1 — BLOCKER: every skill-assist session starts in a never-trusted directory

`repo::skill::new_draft_dir()` (`src-tauri/src/engine/repo/skill.rs:501-509`)
mints `<data_dir>/Conclave/skill-drafts/<fresh uuid>` per session. That exact
path has never been trusted and never can be: `~/.claude.json` has 94 project
entries and **0** under `skill-drafts`. Claude Code therefore blocks on the
trust dialog before reading the prompt, while the frontend marks the session
`running` and locks the editor ("Agent is editing — stop the session to edit
manually", `SkillEditor.tsx:106/119/127`). The dialog's default selection is
**"No, exit"**, so a blind Enter quits the agent.

Falsified alternatives, with evidence:
- *"A permission flag suppresses it."* FALSE. A, B and D all prompt; the
  `--permission-mode` value the app sends is irrelevant, and even
  `--dangerously-skip-permissions` prompts. The trust dialog is orthogonal to
  tool-permission mode, exactly as this plan's Coordination section suspected.
- *"It is keyed on the git root."* FALSE. F sets up an untrusted git repo and
  runs in a nested subdir: it still prompts, and the dialog names the **cwd**,
  not the repo root.
- *"Trust does not reach subdirectories, so a stable parent would not help."*
  FALSE. E and G both pass — a brand-new subdirectory of a trusted ancestor is
  trusted, and G proves the ancestor does NOT have to be a git repo.

**Minimal repair (needs Aoki's ruling — it writes a trust record):** make the
STABLE parent `<data_dir>/Conclave/skill-drafts` trusted once, and every
per-session UUID child inherits it (proven by G). This keeps
`new_draft_dir()`'s per-session isolation exactly as designed — no change to
the concurrency model. It is one app-owned directory, not a broad trust
bypass, but it IS a write to `~/.claude.json`, which this plan puts behind a
ruling. If that write is refused, the alternative is an in-app first-run
consent that performs the same one-time write with the human's explicit OK.
Boundary: `src-tauri/src/engine/repo/skill.rs` (+ wherever the one-time write
lands). **No product edit made — awaiting ruling.**

## D2 — the assist pane never resizes its PTY, so the dialog renders unreadable

`SkillAssistPanel.tsx:98-117` fits the xterm (`FitAddon` + `ResizeObserver`)
but never calls `ipc.session.resize`. `Terminal.tsx:381/401/406` does exactly
that after every fit. So the PTY keeps its spawn size — `pty.rs:56-58` opens
80x24 — while the pane is a fixed `w-[360px]` (~48 columns at fontSize 11.5).

Claude Code positions each word at an absolute column for an 80-column
terminal; every word placed past ~48 clamps to the renderer's last column.
That is precisely the screenshot's scrambled text — the right-edge fragments
reconstruct as `creat|ed` + `o|r` + `o|ne` + `y|ou` + `t|rust?` = "created or
one you trust?". Confirmed against my own raw capture, which contains
`Quick\x1b[8Gsafety\x1b[15Gcheck:` … `\x1b[70G(Like\x1b[76Gyour`.

Consequence: even a user who knows to answer the dialog cannot read it.

**Minimal repair:** call `ipc.session.resize({sessionId, cols, rows})` from the
assist panel's fit path, mirroring `Terminal.tsx`. Boundary:
`src/components/SkillAssistPanel.tsx` — **owned by the planned
terminal-filedrop-paste lane under Detoro. Aoki must coordinate; I did not
touch it.**

## D3 — the assist panel's send never submits

`SkillAssistPanel.handleSend` (`SkillAssistPanel.tsx:174-191`) sends the
bracketed paste and stops. The working reference in the same repo,
`StdinBar.handleSend` (`StdinBar.tsx:105-125`), sends the paste, waits 40ms,
then sends a **standalone `\r`** — with a comment explaining that an embedded
CR is swallowed by TUI paste-burst detection and must arrive separately.
`message::send` (`message.rs:68-99`) only routes stdin; it appends no CR (that
belongs to `inject`, a different function). There is exactly one
`message.send` call in the entire skill-editor path and it has no CR.

So the user's instruction lands in Claude Code's composer and is never
submitted. For `claude` the bootstrap travels via `--append-system-prompt`, so
nothing else submits on its behalf either.

Status: **code-verified, not yet observed end-to-end** — D1 blocks the session
before D3 can be reached, so an end-to-end demonstration has to wait for D1's
repair. Flagging it now precisely so the repair is not declared done after D1.

**Minimal repair:** mirror `StdinBar`'s paste → 40ms → `"\r"` sequence.
Boundary: `src/components/SkillAssistPanel.tsx` — same ownership conflict as
D2.

## Recommended scope

One lane fixing D1 (Rust, `repo/skill.rs`) and one fixing D2+D3 (both are
three-line changes in `SkillAssistPanel.tsx`, and that file belongs to the
planned terminal-filedrop-paste lane — they should either go into that lane or
it should be sequenced first). D1 alone does not restore skill creation.
