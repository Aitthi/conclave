# conclave browser: make click/type reliable so agents stop escaping to Chrome

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Tiësto (e60b9644), AFTER `browser-first-paint` merges (same boundary file) · reviewer: Armin (be81029a) · escalation: Detoro via `task challenge`
base: main after the `browser-first-paint` merge · boundary: `src-tauri/src/engine/runtime/browser.rs`, `src-tauri/src/engine/commands/cli.rs`, `src-tauri/src/bin/conclave-cli.rs`, plus (ruling 3136cc7d, 2026-09-05, credit Tiësto) `src-tauri/src/engine/commands/browser.rs` and `src-tauri/src/engine/router.rs` — the payload seam and the verb registration that Decisions 2 and 5 need

## Request (human, 2026-09-05)

> บางที agent ก็ไปเปิด google chrome ใช้เพราะทำอะไรสักอย่างที่ agent ต้องการไม่ได้ ที่เห็นบ่อยสุดคือสั่ง click ไม่ได้

Agents abandon the in-app browser and open the human's Google Chrome (via the claude-in-chrome MCP or Playwright) because `conclave browser click` "does not work". The standing rule that forbids the fallback is on the blackboard (`protocol:browser-conclave-only`); this lane removes the reasons for it.

## Root causes (Detoro, from code + Dew's findings on task roster-row-supervisor-trash 2026-09-05)

1. `click_js` (`browser.rs:320-337`) does `el.click()`. That fires only a synthetic `click`; React/base-ui/Radix widgets listen on `pointerdown`/`mousedown`, so menus, popovers, toggles and most custom buttons ignore it. Mellow needed Playwright "real clicks" to open the roster popover for the same reason.
2. No wait: `click`/`type` run `querySelector` once. An SPA that has not mounted the element yet returns `{ok:false, message:"no element matched selector"}` immediately.
3. The CLI prints that JSON and exits 0 (Dew: gate script swallowed a missed click). An agent that checks exit codes believes the click happened.
4. `requestAnimationFrame` does not tick in a hidden webview (Dew measured `rafTicks=0` after 1.5 s), so pages driven by rAF (React concurrent features, base-ui transitions, uishot's readiness sentinel) never progress while the agent's tab is not the active one — and an agent's tab is almost never the active one.

## Decisions (Detoro rulings — final)

1. **Click = real pointer sequence at the element's centre.** `click_js` scrolls the element into view (`scrollIntoView({block:"center"})`), computes the centre from `getBoundingClientRect`, and dispatches in order `pointerover, pointerenter, mouseover, pointermove, mousemove, pointerdown, mousedown, focus (if focusable), pointerup, mouseup, click` with `bubbles:true, cancelable:true, composed:true, clientX/clientY, button:0, buttons:1 on down, pointerId:1, pointerType:"mouse", isPrimary:true`. Falls back to `el.click()` only if the dispatch throws. Return shape unchanged (`{ok, url|message}`); add `tag` and `text` (first 80 chars) of the hit element for the agent's log.
2. **Wait-for-selector on `click` and `type`.** Default timeout 5000 ms, poll every 100 ms, done in Rust around `eval_value` (the JS stays a single synchronous IIFE — `eval_with_callback` cannot await a promise). Optional `--timeout-ms N` on the CLI (0 = single attempt). On timeout return `{ok:false, message:"selector not found within 5000ms: <sel>"}`.
3. **CLI exit code follows `ok`.** `conclave browser click|type` exit 1 when the result is `{ok:false}`, JSON still printed to stdout, `message` also on stderr. `open/goto/close/snapshot/eval/screenshot` unchanged. Fix the help line at `conclave-cli.rs:130` to say so.
4. **Hidden tabs must keep ticking — implement as an experiment with a measurable gate.** Replace `view.hide()` for NON-active tabs with "park offscreen": keep the webview shown but `set_position` to `(-20000, -20000)` at its last size, so WebKit still considers it in-window and rAF/timers run at full rate. `set_active` moves the incoming tab to `last_bounds`, parks the outgoing one. `set_visible(false)` parks the active tab instead of hiding it. Gate: from a NON-active agent tab, `conclave browser eval` counts rAF ticks over 1 s → expect > 30. If parking does not keep rAF alive on this macOS/WebKit, file a `task challenge` with the measurement and fall back to hide() + a documented limitation in the CLI help ("your tab is background; rAF-driven pages need `browser status`/`set-active`") — the other three decisions still ship.
5. **`snapshot` reports readiness.** Add `readyState: document.readyState` and `rafAlive: boolean` (measured over 200 ms inside the snapshot JS via a busy-wait-free trick is impossible synchronously — instead expose it as a separate cheap verb `browser ping` returning `{readyState, rafTicksPer200ms}` measured Rust-side with two evals 200 ms apart). Agents get a truthful "is this page alive" before blaming click.

6. **`close_tab` reports the close, not the promotion** (amendment 2026-09-05, Tiësto cross-lane finding on main daa9dcc). After `view.close()` and the registry drop have succeeded, a failed `reveal_at` of the PROMOTED tab must not turn the call into an `Err`: the frontend would show "Couldn't close the tab" (false) and skip `applyState`, leaving the closed tab in the rail until the 2 s poll. Rule: `close_tab` returns `Ok(state())`; the promotion reveal is best-effort and its failure is logged at warn level with the tab id. `reveal_at` itself keeps never-show-after-failed-position (Armin's finding stands); only the propagation from `close_tab` changes. Test: `close_tab_returns_ok_when_promotion_reveal_fails` on the pure decision if reachable, else a comment naming the invariant next to the warn.

## Gates

1. `cargo test -p conclave --lib browser` exit 0 with new tests: `click_js_dispatches_pointer_sequence_before_click`, `click_js_scrolls_into_view`, `wait_for_selector_times_out_with_message` (pure retry helper tested with a fake evaluator), CLI arg tests for `--timeout-ms` and exit-code mapping.
2. `cargo fmt --check` scoped to boundary files; clippy no new warnings.
3. Live gate on the machine (the app must be running; use the lane's own dev page on a free port, not :1420): `conclave browser open http://localhost:<port>/?fixture=default#view=home` → `conclave browser click '[aria-label^="More actions for"]'` → `conclave browser eval 'document.querySelectorAll("[aria-label=\"Remove agent\"]").length'` → expect 1 (the base-ui popover opened from a CLI click — the exact thing that fails today). Then `conclave browser click 'nope'` → exit 1 within ~5 s. Record all three as gates.
4. Decision 4 gate: rAF ticks measured from a background tab, recorded as a gate with the number.
5. Human acceptance after rebuild+relaunch: an agent asked to click something in the in-app browser succeeds without touching Chrome.

## Risks

- Dispatching synthetic pointer events sets `isTrusted:false`; a few sites gate on it (rare; Playwright-via-CDP is the only way around, out of scope).
- Parking offscreen changes `hide()` semantics for the "overlay covers the terminal" fix in `browser-first-paint` — parked tabs are offscreen, so they cannot paint over anything, but the `overlay_visible` gate must still decide whether the ACTIVE tab sits at `last_bounds` or parked. Keep the registry decision functions from that lane and change only what "hidden" means natively.
- `set_position` with large negative coordinates: verify wry/tao accepts it (logical position is f64; if clamped, use `(-width-100, 0)`).

## Deferred

- CDP-level trusted input events; multiple tabs per agent; load-complete signal.

## Outcome

Implemented by Tiësto on `lane/browser-click-reliability`, base main `daa9dcc`.

**Commits:** `daecd77` — D1-D5 (they share `click_js`, the action result shape,
and the conceal/reveal pair; splitting them would produce commits that do not
build). `03ab5fa` — D6, which READY-1 wrongly claimed was in `daecd77`.

**Correction (Armin's finding, Detoro confirmed).** D6 was not in the lane at
READY-1. Its patch ran with the shell still in the MAIN checkout, so it edited
that working tree rather than this worktree, and the grep used to verify it ran
in the same wrong place — so it read as landed. The stray edit has been reverted
and the main checkout is clean; D6 is now in `03ab5fa`. Guard recorded in
workspace memory: `cd <lane worktree> &&` in the same command as any patch
script, and check `git diff --stat` in the lane for every decision claimed
before committing.

**Superseded:** `daecd77` — all six decisions in one change (they share `click_js`,
the action result shape, and the conceal/reveal pair; splitting them would have
produced commits that do not build).

**Gates**

| # | Command | Result |
|---|---|---|
| 1 | `cargo test -p conclave --lib browser` @ `03ab5fa` | exit 0 — 69 passed |
| 2 | `cargo test -p conclave --lib` @ `03ab5fa` | exit 0 — 1045 passed, 11 ignored |
| 2 | `rustfmt --check` on all five boundary files @ `03ab5fa` | exit 0 |
| 2 | `cargo clippy -p conclave --all-targets` @ `03ab5fa` | exit 0 — same 2 pre-existing warnings |
| 1 | `cargo test -p conclave --lib browser` @ `daecd77` | exit 0 — 69 passed |
| 1 | `cargo test --bin conclave-cli` @ `daecd77` | exit 0 — 169 passed |
| 2 | `cargo test -p conclave --lib` @ `daecd77` | exit 0 — 1045 passed, 11 ignored |
| 2 | `rustfmt --check` on all five boundary files @ `daecd77` | exit 0 |
| 2 | `cargo clippy -p conclave --all-targets` @ `daecd77` | exit 0 — 2 warnings, both pre-existing (`instance.rs:2083`, `pty.rs:230`), zero in the boundary files |
| 3 | Live click/popover + exit-code gate | **BLOCKED on rebuild+relaunch** |
| 4 | Background-tab rAF measurement (Decision 4) | **BLOCKED on rebuild+relaunch** |
| 5 | Human acceptance | pending |

Tests named by gate 1 all exist: `click_js_dispatches_pointer_sequence_before_click`,
`click_js_scrolls_into_view`, `wait_for_selector_times_out_with_message` (pure
retry helper driven by a fake evaluator, no webview). Added beyond the named set:
`click_js_reports_the_hit_element`, `click_and_type_report_the_retryable_miss_marker`,
`wait_for_selector_returns_a_late_hit`, `wait_for_selector_zero_timeout_is_a_single_attempt`,
`resolve_selector_timeout_defaults_and_honours_zero`, `ping_scripts_arm_and_read_a_frame_counter`,
`ping_result_deserializes_from_the_read_script_shape`, `browser_click_and_type_accept_timeout_ms`,
`browser_timeout_ms_rejects_a_non_integer`, `browser_ping_maps_with_caller_id_only`,
`browser_click_and_type_exit_non_zero_on_ok_false`, `other_browser_verbs_keep_their_exit_code`.

**Why gates 3 and 4 cannot be run by the implementer.** `conclave browser` drives
the RUNNING app over `conclave.sock`, and the running app is the pre-`daecd77`
build — so a live click gate today would exercise the old `el.click()` and the
old router, which has no `browser.ping` arm. `pnpm tauri dev` cannot take
`conclave.sock` (its janitor prunes the live db), so a second instance is not a
substitute. Both gates need a rebuild + relaunch first; Decision 4's outcome —
including a `task challenge` with the measurement if parking does not keep rAF
alive — is therefore still open, exactly as the plan anticipated.

**Notes on the build**

- **Decision 4 is isolated in one function.** `conceal(view, bounds)` is the only
  place that decides what "hidden" means natively. The documented fallback
  (`view.hide()`) is a one-function change, and Decision 7 of `browser-first-paint`
  still holds either way: a parked webview is at `(-20000, -20000)`, so "nothing
  paints over the app chrome while the Browser view is unmounted" is now enforced
  by POSITION rather than by the hidden flag. The registry decision functions
  from that lane are untouched, per this plan's §Risks.
- **Decision 5's two halves.** The plan's sentence starts by adding `readyState`
  and `rafAlive` to `snapshot`, then rules that the rAF half is impossible
  synchronously and must be a separate `browser ping` verb. Read as: `snapshot`
  gains `readyState` (cheap, synchronous), and `ping` carries both. That is what
  shipped.
- **`BrowserActionResult` gained `tag` and `text`** as `Option` with
  `skip_serializing_if`, so `src/ipc/types.ts`'s mirror keeps deserializing
  unchanged — the frontend reads only `ok`/`url`/`message`. The TS mirror is
  outside this lane's boundary and was deliberately not edited.
- **`isTrusted` is still false** on the synthetic events (plan §Risks). A site
  gating on it needs CDP-level input, which stays out of scope.

