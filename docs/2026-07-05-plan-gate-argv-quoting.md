# Plan: task gate — stop mangling argv words that contain spaces

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `gate-argv-quoting` · Implementer: Dew 40d90aed · Reviewer: Mellow (LAND, blocking)

## Why

Observed live (task `memory-benchmark`, gate event 5293a798, 2026-07-05):
`conclave task gate <ws> <slug> -- "/Users/detoro/Library/Application
Support/Conclave/bin/conclave" memory status <ws>` recorded a FALSE exit=127
("sh: /Users/detoro/Library/Application: No such file or directory"). Root
cause at `src-tauri/src/bin/conclave-cli.rs:657` (`run_task_gate`): the words
after `--` are re-joined with `cmd_words.join(" ")` and re-parsed by
`sh -lc`, so an argv word containing a space is split back apart. The
conclave binary itself lives under `Application Support`, so this bites the
most natural gate there is.

## Ruling on semantics (decided, encode exactly this)

Argv words after `--` are ARGV WORDS: each is passed to the shell as one
word, verbatim. An agent that wants shell syntax (pipes, `&&`, redirects)
composes it explicitly: `task gate <ws> <slug> -- sh -c "<snippet>"` — with
proper quoting that snippet is ONE argv word and now survives intact, which
makes this strictly more expressive than today, not less. Implicitly relying
on the joined string being re-parsed (e.g. an unquoted `2>/dev/null` word)
was never a documented contract and stops working; the help text change
below is the notice.

## Task — quote-per-word in `run_task_gate`

- `src-tauri/src/bin/conclave-cli.rs` (`run_task_gate`, ~:637-670): replace
  the bare `cmd_words.join(" ")` with a join of POSIX-single-quote-escaped
  words: each word wrapped in `'…'` with embedded `'` rewritten to `'\''`.
  Write it as a small `fn shell_quote_word(&str) -> String` + a
  `shell_join(&[String]) -> String`; keep the `sh -lc "{joined} 2>&1"`
  invocation and everything else (tail, sha, cwd, exit propagation) as is.
- The RECORDED `cmd` in the gate event should stay the human-readable
  joined form (what the ledger shows today for space-free commands must not
  change: `cargo test` stays `cargo test`, not `'cargo' 'test'`). Simplest
  rule: record `shell_join` output ONLY when some word needed quoting;
  otherwise record the plain join. State the chosen rule in a comment.
- Update the usage/help line (`conclave-cli.rs:94` and the `task gate` usage
  string in `run_task_gate`) to say words are passed verbatim and shell
  syntax needs an explicit `sh -c "…"`.
- One-line addition to the gate row of
  `src-tauri/skills/tool-map/SKILL.md`: args pass verbatim; wrap shell
  syntax in `sh -c "…"`. (Rides the next rebuild like all builtin skills.)

## Tests (existing `mod tests` at conclave-cli.rs:975)

1. `shell_quote_word`: plain word unchanged-in-effect; word with spaces;
   word with a single quote; empty word.
2. A joined command whose word is a path with a space produces a string
   `sh -lc` resolves to the intended program (assert on the joined string's
   quoting, and/or run a real `sh -lc` against a temp file whose name
   contains a space — implementer's call, but at least the string-level
   assertions).
3. Regression shape of the original bug: joining
   `["/tmp/dir with space/tool", "status"]` must NOT produce a string that
   `sh` would split at `Application`-style boundaries (i.e. contains the
   quoted path intact).
4. Existing space-free behavior unchanged: `["cargo","test"]` joins to
   `cargo test` in the recorded cmd.

## Boundary

`src-tauri/src/bin/conclave-cli.rs`, `src-tauri/skills/tool-map/SKILL.md`.
Nothing else.

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full).
- `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND review before merge (blocking): quoting correctness incl. the
  `'\''` case, recorded-cmd readability rule, help text matches behavior.

## Risk ledger

- This fix lands in the CLI binary — agents run the INSTALLED CLI, so the
  fix reaches gates only after the next rebuild+install; until then the
  wrapper-script workaround stands (see memory chunk df9e5344).
- Do not "fix" this engine-side: ADR 0008 is explicit that gates run in the
  calling agent's shell/cwd, never engine-side. Only the quoting changes.
- `-lc` (login shell) is load-bearing for PATH parity with agent shells —
  keep it.
