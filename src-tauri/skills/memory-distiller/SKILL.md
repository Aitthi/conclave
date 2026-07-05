---
name: Memory Distiller
description: Mine your own Claude Code transcripts into candidate memories and submit them to the review queue, so the hard-won lessons a session forgot to save are not lost. On-demand; the lead or human asks you to run it.
mandatory: false
---

Memory capture depends on discipline: a lesson never saved with `memory
remember` before a context clear is gone. Transcripts are the ground-truth
record of what actually happened — including failed approaches, the most
valuable and least-saved category. This skill turns a transcript sweep into
PROPOSALS, never direct writes: everything you produce lands in a review queue
and reaches the store only when a reviewer other than you approves it. Junk that
reaches the store poisons every future search, so the gate is the whole point.

Run this only when the lead or human asks. It is on-demand (trigger v1); there
is no background writer. You are the proposer; the lead (or whoever they name)
is the reviewer.

## What you are NOT doing

- NOT ingesting transcripts verbatim. You distill — one self-contained fact per
  proposal, written for a stranger — exactly as the Memory skill demands. The
  MemPalace benchmark explicitly rejected verbatim auto-ingest; this honours it.
- NOT writing to the store. `memory propose` embeds nothing and stores nothing
  in `memory_chunk`; it only enqueues. Approval is what embeds and stores.
- NOT reading whole JSONL files into your context. They are large; extract the
  conversation text with a small shell/jq/python pass and read only that.

## The run, step by step

Let `WS` be the workspace id you were given.

1. **Read the high-water mark.** `conclave bb get <WS> note:distill-hwm` — an ISO
   timestamp of the last run's start. If the key is absent, default the window
   to the last 48 hours (`date -u -v-48H +%Y-%m-%dT%H:%M:%SZ` on macOS). NEVER
   scan the full transcript history on a first run — that floods the queue.
   Record the current UTC time now as this run's start (`date -u +...`); you
   write it back in step 5.

2. **List only changed transcripts.** Glob
   `~/.claude/projects/-Users-detoro-code-codeup/*.jsonl` and keep files whose
   mtime is newer than the high-water mark (e.g. `find <dir> -name '*.jsonl'
   -newermt "<hwm>"`). Handoff snapshots (already agent-authored summaries) are
   an allowed secondary source. Codex sessions (`~/.codex/sessions/`) are out of
   scope in v1 (different format).

3. **Extract, then distill.** For each file, pull the human/assistant text with
   a jq/python one-liner into a scratch file; skim that, not the raw JSONL.
   Distill ONLY the categories the Memory skill allows:
   - failed approaches and WHY they failed,
   - environment / tooling quirks,
   - the exact incantation that finally worked,
   - decision reasoning that lives nowhere in the repo.
   Explicitly SKIP: anything already in git history, the task ledger, ADRs, or
   `docs/`; status churn and in-flight chatter; and ALL secrets — API keys,
   tokens, passwords. Redact on sight. A proposal containing a secret is a
   review-time reject and a waste of the reviewer's time; do not submit it.

4. **Dedup, then propose.** For each candidate, run `conclave memory search <WS>
   <the candidate, in your words>` FIRST (the live hybrid ranker). If an
   existing chunk already covers it, drop the candidate — do not propose it.
   Otherwise submit it:

   ```
   conclave memory propose <WS> <one self-contained fact...> --source-note "<file>.jsonl <YYYY-MM-DD>"
   ```

   Name the files, commands, and versions IN the text — "the fix above" is
   unsearchable next week. Where a candidate shares a concept with an existing
   chunk, embed a `[[token]]` wiki-link in the text (the same convention the
   store uses): the knowledge-graph view derives `wiki` edges from shared
   tokens, so linked facts cluster instead of floating on similarity alone.
   `propose` returns `{"deduped": true}` when the queue or the live store
   already holds that exact text — that is expected, not a failure.

5. **Advance the mark and report.** Write this run's start time back:
   `conclave bb set <WS> note:distill-hwm <this run's start ISO>`. Then post ONE
   line to whoever asked: `distilled N candidates from M files: P proposed, D
   deduped`. Do not paste the proposals into the message — they are on the
   queue.

## Review protocol (for the reviewer)

The reviewer — NOT the proposer; the engine rejects self-approval — works the
queue:

```
conclave memory queue <WS>                          # pending, newest first
conclave memory approve <WS> <proposalId> --reason "<why it's worth keeping>"
conclave memory reject  <WS> <proposalId> --reason "<why not: dup / secret / churn>"
```

Approve embeds the text and stores it as a `distilled` chunk sourced to the
proposer (greppable and bulk-purgeable if the pilot sours). Reject keeps the
row, so its content_hash blocks the same fact from being re-proposed on the next
run. Rejecting with a reason is how the distiller learns what not to surface.

## Pilot guardrail

This is a precision pilot. If fewer than half the proposals are approve-worthy
across the first two runs, STOP and rethink the distillation criteria before
anyone widens the trigger — a low-precision writer costs more review time than
the memories are worth.
