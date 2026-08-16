# Trailing-system guard: pin the combined walk-back and cover the H2 builders

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

Follow-up from Mellow's post-merge audit of `h1-generation-400` (merged
f9fc1b5). Two accepted challenges, both about the same API rule Dew
live-verified: a mid-conversation `{"role":"system"}` message must end
`messages` or be followed by an `assistant` turn, so appending a user turn
after an arbitrary prefix/original can 400.

## Item 1 — pin the combined walk-back (challenge fd2194f3, accepted)

`ct::prefix_messages_before_appended_user` uses ONE combined loop
(tool-closure + trailing-system checked together each step). Mellow's
mutation run proved a sequential two-pass refactor (prefix_messages, then
trim trailing system) passes all five merged tests while emitting a prefix
with a DANGLING tool_use. Separating fixture:
`[user, assistant(tool_use tu_a), system(tool_result tu_a), assistant]` —
shipped fn walks to len 1 (closed); the two-pass mutant stops at len 2 with
`tu_a` unmatched.

Add ONE unit test in `count_tokens.rs` using that fixture: assert the result
has len 1 AND passes `tool_exchanges_are_closed`. Mellow's scratch crate is
reusable as reference:
`/private/tmp/claude-501/-Users-detoro-code-codeup/9bb1511a-efe3-491f-a4a0-f0a501909d74/scratchpad/prefixmut`.
Reachability today is low (Claude Code system messages carry text blocks) —
this is durability, not a live defect.

## Item 2 — the H2 quality builders are unguarded (challenge 67070896, accepted)

The H1 fix covered the H1 generation body only. Same pattern still live in
H2 (never yet run against a real conversation — exactly how H1's 400 hid):

- `src-tauri/src/engine/runtime/quality.rs:804-806` `build_replay_call`
  pushes `{"role":"user", REPLAY_INSTRUCTION}` onto a caller-supplied
  messages array — called twice per case at `ctx_proxy.rs:3179-3182`, with
  `candidate.original_messages` and with `projection.projected_messages`
  (whose tail is the original's tail).
- `src-tauri/src/engine/runtime/quality.rs:831-845` `build_judge_call` does
  the same on the full original messages.

Any original ending on a system message ⇒ all three calls 400
(`invalid_request_error`), the row-4 failure class.

Fix: extract the trailing-system(+tool-closure) guard into ONE shared helper
(natural home: `count_tokens.rs` beside `prefix_messages_before_appended_user`
— reuse its walk-back rather than duplicating it) and apply it before the
instruction push at `quality.rs:805` and `:841`. Add a mock-upstream
regression test per builder reusing the `EnforceSystemBoundary` rule already
written at `ctx_proxy.rs:4522` (fixture: original messages ending on a
system message; assert the call bodies stay legal and the case completes).

Semantics note: for the H2 REPLAY call, dropping a trailing system message
changes what is replayed; walking back (dropping) is the accepted semantics
per the H1 precedent — record in a code comment that the replay/judge input
is the guarded prefix, and keep A/B comparability by guarding BOTH the
original-side and projected-side calls identically.

## Item 3 — narrow the over-claimed "live-verified" comment (accepted)

The local rule-checker copies (`count_tokens.rs:~419`,
`ctx_proxy.rs:~4519`) say "Verified against the live API", but only the
successor half of the rule was captured live; the `index == 0 => reject`
clause has no recorded probe. Also the walk-back inspects `arr[end-1]` only,
so a LEADING system message would pass through untouched (harmless — such an
original is rejected upstream before H1 ever sees it, and prefix adjacency
preserves interior legality). Ruling: narrow both comments to claim live
verification for the successor clause only, and note the leading-system
behavior at the walk-back. No live probe required.

## Recorded notes (challenge on C-vs-gen drift, accepted — no code action)

Mellow traced the drift end to end: arithmetically SAFE — c/r/s_h come from
the unchanged cache prefix, g from measured usage, so the shortened gen
prefix cannot corrupt R, S_h, q_h, or n_h. Two residuals recorded here:
(a) sources are inlined by `render_untrusted_sources`, so a shorter prefix
costs generation context, not fidelity — this is what makes the narrow fix
safe; (b) a below-cache-breakpoint gen prefix trades cache_read for input,
raising g and therefore n_h — a conservative bias in the economics, now
documented. The gen-prefix observability column stays DEFERRED (owner ruling
in the h1-generation-400 plan), reinforced by (b) being conservative.

## Constraints

- Do not change `prefix_messages` (C/cache authority) semantics.
- H1/H2 privacy non-goals still bind (no body persistence).
- This is a pre-H2-arm blocker: H2 `quality-shadow` must not be armed before
  this lane merges (owner will hold arming).

## Boundary

- `src-tauri/src/engine/runtime/count_tokens.rs` (helper + item-1 test)
- `src-tauri/src/engine/runtime/quality.rs` (both builders)
- `src-tauri/src/engine/runtime/ctx_proxy.rs` (call sites if needed + tests)

## Gates before READY

- `cd src-tauri && cargo test engine::runtime::count_tokens`
- `cd src-tauri && cargo test engine::runtime::quality`
- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `git diff --check`
- Record via `conclave task gate` per standing protocol.
