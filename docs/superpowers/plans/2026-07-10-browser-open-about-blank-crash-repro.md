# browser open about:blank — app-crash repro (repro-only, no fix)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Evidence (task inapp-browser-agent-tools-v3 note 87cab08f, 2026-07-10)

The Conclave app died at ~04:50:30Z during Tiësto's compound probe chain:
`browser open about:blank; browser goto https://example.com; browser eval
"<multi-statement>"; browser type '#t1' hello world; browser eval ...; browser
close`. No tool result returned (transcript 46263d6f ends right after the
tool_use), so the killing verb is not isolatable from the transcript. Every
verb in that chain EXCEPT `open about:blank` re-passed individually today from
an https start. The multi-statement eval in the chain is now known NOT to
execute at all (separate task browser-eval-multi-statement) — which further
narrows suspicion to the non-http scheme open.

## Scope (deliberately narrow — workspace standard: wait for live repro before hardening)

1. At a moment when NO other lane is in flight (a crash kills every agent
   session), run exactly: `conclave browser open about:blank` — one command,
   nothing compound.
2. If the app survives: run `conclave browser status`, then `close`, note the
   results, and close this task as not-reproduced (the evidence stays on the
   ledger for the next occurrence).
3. If the app crashes: capture the macOS crash report
   (~/Library/Logs/DiagnosticReports, newest Conclave entry) BEFORE restarting,
   attach the path + crashing thread to the task note, and STOP — the fix gets
   its own planned task with the crash report as its evidence.

## Boundary

- (none — repro-only; no source edits under this slug)

## Risk ledger

- Reproducing may kill all running agent sessions — that is why step 1 is gated
  on an empty in-flight board. Lead runs this personally.
