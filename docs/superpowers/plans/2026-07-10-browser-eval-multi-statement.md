# browser eval: support multi-statement input (or fail loudly, never silently)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem (Tiësto's live-verify finding, task inapp-browser-agent-tools-v3 note 87cab08f, 2026-07-10)

`conclave browser eval "<js>"` with multi-statement input (semicolon-separated,
e.g. `document.body.innerHTML += '<input id=t1>'; 'injected'`) fails with
`internal: browser webview error: eval result was not JSON (EOF while parsing a
value at line 1 column 0)` and the statements DO NOT execute. Single-expression
eval works.

## Root cause (confirmed by lead, browser.rs read 2026-07-10)

`eval_js` (src-tauri/src/engine/runtime/browser.rs:370) wraps agent JS as
`return ({js});` — a parenthesized *expression*. Multi-statement input makes the
ENTIRE wrapper IIFE a parse-time SyntaxError, so the in-page try/catch never
runs, `eval_with_callback` fires with an empty string, and
`eval_value` (browser.rs:409) surfaces the serde EOF error. The `{__error}`
contract (module doc line 14-16) is bypassed entirely.

## Fix (settled design — do not re-litigate; escalate only with new evidence)

1. Rewrite `eval_js` to construct the function INSIDE the page, source passed as
   a JS string literal via the existing `js_literal` helper:
   - expression-first: `new Function('return (' + src + '\n)')`
   - on SyntaxError fall back to indirect eval — `(0, eval)(src)` — because only
     `eval()` has completion-value semantics; a `new Function(src)` body without
     an explicit `return` always yields `undefined`, which fails this plan's own
     acceptance matrix (`document.title; 42` → 42). Construction and execution
     stay SEPARATED: `new Function(src)` is used purely as a parse check before
     the eval runs, so a RUNTIME throw on the expression path can never fall
     through and re-execute side effects via eval.
     [Amended 2026-07-10 per challenge 92aa7d2b (Tiësto) — the original fallback
     `new Function(src)` was proven wrong by a node harness running the
     byte-identical wrapper template; upheld verbatim.]
   - call it inside the existing try/catch; normalize `undefined` results to
     `null` before returning (`var r = fn(); return r === undefined ? null : r;`)
   - any construction/execution error still returns `{ __error: String(...) }` —
     the contract is unchanged.
2. In `eval_value`, map an empty/whitespace `raw` to a CLEAR error
   (`"eval produced no result (script failed to parse?)"`) instead of the raw
   serde EOF text — defense in depth if some future wrapper regresses.
3. Unit tests beside `eval_js_wraps_expression_in_try_catch` (browser.rs:832):
   - single expression still produces the expression-first wrapper
   - multi-statement source survives `js_literal` embedding (quotes, newlines)
   - (rust-side string tests only; live behavior verified at the gate below)
4. Live-verify matrix — INTEGRATOR step, not implementer: the running app
   carries the pre-fix engine, so this can only run after the next app
   rebuild/relaunch (flagged by Tiësto with challenge 92aa7d2b). Implementer
   substitutes a node harness against the byte-identical wrapper template and
   records it as a gate. Matrix to run post-rebuild:
   - `conclave browser eval "1+1"` → 2 (regression)
   - multi-statement: `conclave browser eval "document.title; 42"` → 42
   - statement-only: `conclave browser eval "var x = 5; x * 2"` → 10
   - syntax error: `conclave browser eval "].bad"` → `{__error: ...}`, not EOF

## Boundary

- src-tauri/src/engine/runtime/browser.rs (only)

## Gates

- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --manifest-path src-tauri/Cargo.toml browser
- git diff --check

## Risk ledger

- `new Function` inside a natively-evaluated script: WKWebView native
  evaluateJavaScript is not subject to page CSP for the injected code; if a
  CSP-strict page still blocks the constructor, the error now surfaces as
  `{__error}` — strictly better than today's silent EOF. If live-verify finds a
  page where expression eval worked before but fails after, STOP and escalate.
- Release note: the fix goes live only at next app rebuild/install (installed
  CLI talks to the running engine — engine-side change).
