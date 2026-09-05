# conclave browser: eval must wait for the first navigation commit

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Problem (evidence: Tiësto triage note eecb4a2f on task browser-click-reliability)

`conclave browser snapshot` / `eval` / `click` on a freshly opened tab can fail with the
oneshot `RecvError` message from `browser.rs:923` because the eval callback was dropped
upstream: wry-0.55.1 `wkwebview/mod.rs:721-723` queues the script and DROPS the callback
while `pending_scripts` is `Some` (webview created, first `didCommitNavigation` not fired);
`navigation.rs:29-35` replays without a completion handler. Our `browser.rs:720-723` clears
`loading` at dispatch time, so the JSON's `loading=false` is not evidence of a commit, and
`open` → `snapshot` races the first commit. A target that never commits (dev server down)
leaves the tab permanently broken with no timeout and a message that reads as an engine bug.
Hit live by Hardwell (workspace-overview-canon note d70762da). Separate from f8e6e2e (D1–D6).

## Decisions

- D1 Track first commit per tab with `WebviewBuilder::on_page_load` (`PageLoadEvent::Started`
  = didCommitNavigation). The registry's `loading` becomes honest: set at `open`/`goto`,
  cleared on the first Started event. Contract shape unchanged (field already exists).
- D2 `eval_value` awaits first-commit inside the EXISTING `EVAL_TIMEOUT` (10 s, `browser.rs:43`)
  before dispatching; a tab that never commits fails through the Timeout arm with the message
  `page has not committed a navigation yet (target unreachable?) — check conclave browser status`.
- D3 On the `RecvError` arm (upstream drop that D2 did not prevent, e.g. stale webview id,
  tauri-runtime-wry `lib.rs:3745-3751`): retry once after 250 ms, then fail with a message that
  names the real cause and remedy, never a bare channel error.
- D4 Binary-free tests only (same rule as `commands/instance.rs`): the first-commit gate and the
  retry arm are factored so they run under `cargo test -p conclave --lib browser` with a mocked
  dispatcher. Native WKWebView behaviour is verified live after the human rebuild.
- D5 If, live, the drop reproduces on a tab that HAS committed, path (b) (stale webview id) is
  the live cause: add a webview-liveness precheck instead of widening D2. Record which path
  reproduced in the READY note.

## Boundary

`src-tauri/src/engine/runtime/browser.rs` only. If D1 needs the builder call site in
`commands/browser.rs` or `router.rs`, file a challenge naming the exact line before editing.

## Gates (record each with `conclave task gate`)

```
cargo test -p conclave --lib browser
cargo test -p conclave --lib
rustfmt --edition 2021 --check src/engine/runtime/browser.rs
cargo clippy -p conclave --all-targets
```

(`cargo fmt` on the whole tree is red on main — gate fmt on the lane file only.)

## Done

READY note: lane head SHA, which upstream path (a/b/c) the fix covers, the new error strings
verbatim, and the live re-test plan for after the rebuild (`browser open` → immediate
`snapshot` on a slow page; `open` on a refused port must fail within 10 s with the D2 message).
Reviewer: Armin. Escalation: Detoro (30fa04f4).
