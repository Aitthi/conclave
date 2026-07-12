# Hybrid LLM Summary H1: Spend-Gated Shadow Economics
<!-- conclave-plan:v1
{
"owner":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace","authority":"in-loop",
"planPath":"docs/superpowers/plans/2026-07-12-hybrid-h1-shadow-economics.md","baseSha":"1465d6dee58ca9aca2043b831c787b9a33b3b9ac","escalation":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace",
"readingOrder":["docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md#11-hybrid-llm-summary-proposal-review-gated-no-build-authorized","docs/superpowers/plans/2026-07-12-hybrid-h1-shadow-economics.md","src-tauri/crates/ctxopt/src/summary.rs","src-tauri/src/engine/runtime/count_tokens.rs","src-tauri/src/engine/runtime/ctx_proxy.rs"],
"boundary":["src-tauri/crates/ctxopt/src/summary.rs","src-tauri/src/engine/runtime/summary.rs","src-tauri/src/engine/runtime/count_tokens.rs","src-tauri/src/engine/runtime/ctx_proxy.rs","src-tauri/src/engine/runtime/mod.rs","src-tauri/src/engine/repo/proxy_summary_metric.rs","src-tauri/src/engine/repo/mod.rs","src-tauri/src/engine/migrations/0024_proxy_summary_metric.sql","src-tauri/src/engine/db.rs","src-tauri/src/engine/commands/proxy.rs","src-tauri/src/engine/commands/cli.rs","src-tauri/src/engine/router.rs","src-tauri/src/bin/conclave-cli.rs"],
"consumes":["src-tauri/crates/ctxopt/src/summary.rs#plan_summary_span","src-tauri/crates/ctxopt/src/summary.rs#build_summary_projection","src-tauri/crates/ctxopt/src/summary.rs#render_untrusted_sources","src-tauri/src/engine/runtime/count_tokens.rs#prefix_messages","docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md#116-ordered-live-measurement-plan-and-gonogo-bars"],
"produces":["src-tauri/src/engine/runtime/summary.rs#generate_summary","src-tauri/src/engine/runtime/ctx_proxy.rs#SummaryJob","src-tauri/src/engine/repo/proxy_summary_metric.rs#SummaryReport","proxy.summaryShadow","proxy.summaryReport"],"gates":["cargo test --manifest-path src-tauri/crates/ctxopt/Cargo.toml","cd src-tauri && cargo test engine::runtime::summary","cd src-tauri && cargo test engine::runtime::ctx_proxy","cd src-tauri && cargo test engine::repo::proxy_summary_metric","cd src-tauri && cargo test engine::commands::proxy","git diff --check"]
} -->

## Goal and authorization boundary

Implement H1 **shadow economics only**: after a successful real `/v1/messages`
forward, an explicitly armed off-path sampler may generate an aggregate summary,
build the H0 projection, count A/B/C, charge the actual generation usage against
the human-supplied price schedule, and persist `q_h`/`n_h`. It must never alter,
delay, replace, or retry the forwarded request.

Human authorization on 2026-07-12 funds building H1 instrumentation. It does
**not** authorize generation spend merely because the app was rebuilt. A human
must separately arm one in-memory campaign with the exact CLI command in
§“Spend gate and CLI.” Restart returns to OFF and forgets the price schedule.

H1 ends with an economics report. It does not run H2 faithfulness verification,
behavioral replay, a live apply trial, or any rewrite mode.

## Design-of-record and binding rulings

- Spec authority: `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md`
  §11.5 and §11.6 H1, plus R9.
- H0 authority: commit `1465d6d`, especially
  `ctxopt::summary::{plan_summary_span, render_untrusted_sources,
  build_summary_projection}`. H1 does not reimplement selection or structural
  validation in the runtime.
- Closure authority: runtime `count_tokens::prefix_messages`. H0's private
  `closed_prefix_end`/`SummaryPlan.count_prefix_end` is a fixture drift tripwire
  only and must never supply C or the generation prefix.
- Ruling `35f172a8`: H1 uses `request.model` as the summarizer model. No cheap
  model selector and no model override CLI.
- Ruling `0807943e`: “no tools” means **tool invocation is impossible** via
  `tool_choice: {"type":"none"}`. Preserve original model/system/tools and
  their cache-control bytes for cache identity. Changing `tool_choice` is
  expected to invalidate cached message blocks while system/tools may remain
  cached; record actual cache-read and cache-creation usage separately and make
  no cache-hit claim.
- Ruling `d631ccf5`: arming atomically installs a complete versioned price
  schedule. Missing/invalid prices fail closed before any generation call.
  Standard versus long-context rates are selected per request from measured
  total input tokens, not from bytes or a campaign-wide guess.

## Non-goals

- No forwarded-body mutation, even when a candidate passes every H1 bar.
- No summary reuse on a later forward; H1 only measures a projection.
- No verifier, judge, source-probe recall, human-quality workflow, or H2 schema.
- No persistence of request bodies, source text, summaries, prompts, headers,
  credentials, upstream error bodies, or upstream `error.message`.
- No automatic arming after migration, restart, command replay, or restoring
  prior app state.
- No synthetic “GO” from 100k calibration traffic. The 400–500k real-token band
  remains binding.
- No changes to `ctxopt` closure logic. The separate planned task
  `hybrid-summary-min-size-floor` may refine candidate selection before H1
  integration, but runtime code still consumes the public H0 planner.

## Global constraints (every implementation lane inherits these)

1. **Shadow-only invariant:** the only bytes sent to the original upstream
   forward are the current `upstream_body`. No summary function returns a body
   to `forward_inner`; summary scheduling happens only after a successful
   upstream response, alongside the existing M1 sampler.
2. **Spend invariant:** the sole generation call site begins with a captured
   `SummaryCampaignConfig` from an armed runtime epoch. OFF/default, missing
   config, model mismatch, expired epoch, queue refusal, or parse/plan/count
   failure performs zero `/v1/messages` generation calls.
3. **Off-path invariant:** use `try_acquire_owned`, never `.acquire().await`, in
   the forwarder. All count/generation/projection/database work occurs in a
   spawned task holding its permit. Timeout or failure cannot affect the already
   returned upstream response.
4. **Credential containment:** capture the exact upstream and allowlisted
   credential from the forwarded request. Use a dedicated no-redirect client,
   sensitive header values, explicit timeout, and no retries. Never re-read the
   global upstream inside the job.
5. **Token authority:** `bytes/4` is only a generous admission trigger. A,
   B_h, C, R, S_h, low-water classification, price tier, and `n_h` use provider
   counts/usage. Every admitted job produces one terminal metric row.
6. **Structure authority:** the projection must be `SummaryOutcome::Applied`
   and must pass the existing `ctxopt::validate` path inside H0. Any original/
   rejected outcome persists a bounded label and stops before B/C claims.
7. **Privacy:** persist only bounded labels, counts, versions, and SHA-256
   fingerprints. Error stage/type values come from fixed allowlists. No bodies,
   summary strings, request text, or credentials enter SQLite or logs.
8. **No real model calls in tests or CI:** every `/v1/messages` generation path
   uses a local schema-aware mock upstream. Tests must assert hit counts.
9. **No UI scope:** this milestone changes CLI/runtime/repository code only;
   the standing UI pixel gate does not apply.

## Runtime flow

### Admission in `forward_inner`

After the original `/v1/messages` upstream returns a success status:

1. Read `summary_campaign: RwLock<Option<SummaryCampaignConfig>>`. `None` means
   OFF. Clone the config and current `summary_epoch`; never wait on the lock.
2. Require exact `request.model == config.model`. A mismatch increments
   `summaryModelMismatch` and performs no count or generation call.
3. Apply the existing generous byte trigger `est_tokens(body.len()) > ceiling`.
   It controls only whether to enqueue work. It makes no token/economics claim.
4. Parse the original JSON and capture: original body as `Value`, headers as
   `CountCredential`, exact upstream, model, ceiling, low-water, price schedule,
   campaign id/epoch, and byte diagnostic.
5. Call `try_begin_summary_sample()`: one dedicated permit, a 60-second global
   cooldown, and `try_acquire_owned`. Refusals increment
   `summarySamplesDropped`. Do not share M1's semaphore/cooldown because enabling
   M1 must neither authorize nor starve paid H1 work.
6. Spawn `sample_summary`. The forwarder continues immediately. Keep existing
   M1 sampling independent; `proxy checkpoint on` is neither required nor
   sufficient to authorize H1 spend.

### `SummaryJob` contract in `ctx_proxy.rs`

Use a distinct type, not an extension of `CheckpointJob`:

```rust
struct SummaryJob {
    campaign_id: String,
    campaign_epoch: u64,
    model: String,
    upstream: String,
    credential: CountCredential,
    original: Value,
    ceiling: usize,
    low_water: usize,
    tail_tokens: usize,
    byte_estimate: usize,
    price: SummaryPriceSchedule,
}
```

Credentials remain in memory only and must not implement `Debug`, `Serialize`,
or `Deserialize`. The job does not contain forwardable bytes and no function in
the summary path returns bytes to `forward_inner`.

### Ordered shadow pipeline

`sample_summary` executes these stages in order and persists exactly one row:

1. **Epoch check:** immediately before any network call, verify the campaign is
   still armed at the captured epoch. Otherwise persist `disarmed` with zero
   generation calls. `off` cannot retract a request already sent on the wire;
   status exposes `summaryInFlight`, and this limitation is documented in CLI
   output/help.
2. **Count A:** reuse `count_tokens_body` and `count_tokens`. On failure persist
   `count_failure/count_a`. If real A ≤ ceiling, persist `below_ceiling` and do
   not generate.
3. **Derive a token-sized recent tail:** target the pinned
   `SUMMARY_TAIL_TOKENS = 100_000`. Find the latest message index whose runtime-
   closed prefix leaves at least 100k tokens (`A - prefix_count >= 100k`). Use a
   bounded binary search over message indices; every probe first calls
   `count_tokens::prefix_messages`, then counts that valid prefix. Memoize counts
   by returned prefix length so rolled-back parallel cycles are counted once.
   Cap at 10 probes. On no valid boundary/count failure persist
   `tail_boundary_failure`; never fall back to `tail_msgs` or `bytes/4`.
4. **Plan:** call `ctxopt::summary::plan_summary_span(messages, tail_start)`.
   Persist `no_candidate` on its bounded error enum; do not generate. Ignore
   `plan.count_prefix_end` for runtime accounting.
5. **Build the generation request:** recompute C-prefix messages with
   `count_tokens::prefix_messages(messages, plan.earliest_changed_msg_index)`.
   Preserve original `model`, `system`, `tools`, and their nested
   `cache_control` bytes. Set `tool_choice={"type":"none"}`, omit
   `stream`/`metadata`, set `max_tokens=SUMMARY_MAX_OUTPUT_TOKENS` (8,192), and
   use messages `[...C-prefix, summary-instruction]`. The final user instruction
   contains `render_untrusted_sources(&plan)` as one JSON envelope plus the
   pinned factual summary contract from spec §11.1. This avoids copying raw
   sources into logs while treating tool-controlled strings as JSON values.
6. **Generate once:** recheck epoch, then call
   `runtime::summary::generate_summary`. Accept text blocks only, concatenated in
   response order with `\n`; any `tool_use`, non-text block, empty text, missing
   usage, timeout, redirect, non-2xx, or decode error persists a bounded
   `generation_failure/<stage>` row. No retry.
7. **Project and validate:** call `build_summary_projection`. Require
   `SummaryOutcome::Applied`; otherwise persist
   `projection_rejected/<SummaryError variant>`. Record the H0 byte diagnostics,
   checkpoint id, source count, source-boundary hash, and SHA-256 of the accepted
   summary, but never the summary itself.
8. **Count B_h and C:** B_h counts the full original request with projected
   messages. C counts the runtime prefix already derived in step 5. Label count
   errors `count_b` or `count_c`. Never use H0 `count_prefix_end`.
9. **Compute:** `R=A-C`, `S_h=A-B_h`, `q_h=S_h/R`. Reject arithmetic-invalid
   rows (`R=0`, `S_h=0`, `S_h>R`) as `metric_invalid`. Select the forwarded-turn
   price tier from A and the generation price tier from measured total summary
   input (`input + cache_creation + cache_read`) versus
   `longContextThreshold`. A total **strictly greater than** the threshold uses
   long-context rates; a total equal to or below it uses standard rates. Compute
   `C_gen` and:

   `delta0 = p_w_forward*(R-S_h) - p_r_forward*R + C_gen`

   `n_h = max(0, delta0/(p_r_forward*S_h))`

10. **Persist measured row:** outcome `measured`; record whether `B_h <= L` and
    `n_h <= 2`. Plateau is derived transactionally from the previous row with
    the same campaign/conversation/source-boundary fingerprint: same boundary
    increments, changed boundary resets to zero.

## `runtime/summary.rs` interface and security contract

Add the module to `runtime/mod.rs`. It owns generation networking only:

```rust
pub const SUMMARY_METHOD_VERSION: &str = "h1-shadow-summary-v1";
pub const SUMMARY_PROMPT_VERSION: &str = "aggregate-tool-results-v1";
pub const SUMMARY_MAX_OUTPUT_TOKENS: u32 = 8_192;

pub struct SummaryUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

pub struct GeneratedSummary {
    pub text: String,
    pub response_model: String,
    pub usage: SummaryUsage,
}

pub async fn generate_summary(
    client: &reqwest::Client,
    upstream: &str,
    credential: &CountCredential,
    body: &Value,
) -> Result<GeneratedSummary, SummaryClientError>;
```

`SummaryClientError` exposes only fixed stage/type labels. Refactor
`count_tokens.rs` minimally so summary generation reuses sensitive-header
construction and the allowlisted Anthropic `error.type` sanitizer without
appending `token-counting-2024-11-01` to the generation call. Generation uses
`POST /v1/messages`, preserves the captured `anthropic-beta` value/order, has a
60-second timeout, follows no redirects, and makes no retry. A response-model
mismatch is recorded but does not replace the request model for pricing.

Mock tests must cover: API-key and Bearer credentials; beta preservation without
counting-beta injection; redirect refusal; timeout; status allowlist; hostile
error messages/body markers absent from the returned error; text-only parsing;
tool-use/non-text rejection; missing usage; and all four usage buckets.

## Spend gate and exact CLI

Add IPC `proxy.summaryShadow`, report IPC `proxy.summaryReport`, router entries,
CLI mapping/help, status fields, and bin usage text.

`ProxyRuntime` exposes narrow non-async methods for the command lane:
`arm_summary(config) -> Result<SummaryStatus, SummaryConfigError>`,
`disarm_summary() -> SummaryStatus`, `summary_status() -> SummaryStatus`, and
`snapshot_summary_campaign() -> Option<(u64, Arc<SummaryCampaignConfig>)>`.
Commands never mutate individual atomics/locks or assemble a partial config.

OFF is:

```text
conclave proxy summary-shadow off
```

ON is one atomic command; every flag is required and values must be finite,
non-negative decimal USD-per-million-token rates (cache-read must be >0 because
it is the denominator of `n_h`):

```text
conclave proxy summary-shadow on \
  --model <exact-request-model> \
  --price-version <immutable-version-label> \
  --standard-input-usd-per-mtok <rate> \
  --standard-cache-write-usd-per-mtok <rate> \
  --standard-cache-read-usd-per-mtok <rate> \
  --standard-output-usd-per-mtok <rate> \
  --long-context-threshold <tokens> \
  --long-input-usd-per-mtok <rate> \
  --long-cache-write-usd-per-mtok <rate> \
  --long-cache-read-usd-per-mtok <rate> \
  --long-output-usd-per-mtok <rate>
```

The command parser rejects missing/duplicate/unknown flags, empty model/version,
zero threshold, non-finite numbers, negative rates, and zero cache-read rates.
Only after validation succeeds does the handler create a random `campaignId`,
increment `summary_epoch`, store the complete config, and return status. Any
validation error leaves the prior OFF/ON state unchanged. `off` first increments
the epoch, then clears config, preventing queued-but-unsent jobs from spending.
Restart constructs `None`; no DB row or preference restores it.

`proxy status` adds: `summaryShadow` (bool), `summaryCampaignId`, configured
model, price version and rates, tail/max-output constants,
`summarySamplesDropped`, `summaryModelMismatch`, and `summaryInFlight`. It never
shows credentials or prompt/source data.

Report command:

```text
conclave proxy summary-report [--since-hours N] [--campaign-id ID]
```

It is read-only and never arms generation.

## Persistence: migration 0024 and repository

Create `0024_proxy_summary_metric.sql`, register user_version 24 in `db.rs`, and
add `repo::proxy_summary_metric`. Use nullable numeric fields for stages that
fail before a value exists; never encode missing values as zero.

Required columns:

- identity: `id`, `created_at`, `campaign_id`, `conversation_hash` (SHA-256 of
  the first message bytes), `model`, `response_model`, `method_version`,
  `prompt_version`, `price_version`;
- boundary: `checkpoint_id`, `source_boundary_hash`, `summary_hash`,
  `earliest_changed_msg`, `runtime_prefix_messages`, `tail_start_msg`,
  `tail_target_tokens`, `source_count`, `protected_result_count`;
- counts: `a_tokens`, `b_tokens`, `c_tokens`, `r_tokens`, `s_h_tokens`, `q_h`,
  `summary_tokens`, `tombstone_tokens`, `projected_post_tokens`,
  `byte_est_tokens`, `tail_count_calls`, `plateau_turns`;
- generation usage: `gen_input_tokens`, `gen_cache_creation_tokens`,
  `gen_cache_read_tokens`, `gen_output_tokens`;
- economics: all standard/long price rates, `long_context_threshold`,
  `forward_price_tier`, `generation_price_tier`, `c_gen_usd`, `n_h`,
  `meets_low_water`, `meets_two_turn`;
- terminal state: `outcome`, `failure_stage`, `error_type`.

Index `(created_at)`, `(campaign_id, conversation_hash)`, and
`(campaign_id, outcome)`. Check constraints restrict outcome/stage/tier labels
to allowlists and booleans to 0/1. `insert_terminal` is the only write API and
requires exactly one outcome. A repository test scans text columns after hostile
mock errors and source markers to prove no body/summary content was persisted.

`SummaryReport` returns campaign-aware distributions and the exact H1 numerator/
denominators: total admitted; terminal outcomes; successful candidates;
distinct conversations; count/generation failure rate; candidates in the
400k–500k A band; percent with B≤L; percent with n_h≤2; q_h and n_h
min/median/max/average; price-version/model grouping; cache bucket totals; max
plateau; dropped/model-mismatch counters from status are reported separately.
No pooled percentage is allowed to hide an all-miss conversation: include a
per-conversation pass-count distribution.

## Implementation slices and ownership boundaries

Detoro creates the downstream tasks and assigns implementers. Do not claim this
plan task as an implementation lane.

### Lane A — generation client (independent first wave)

Boundary: `runtime/summary.rs`, `runtime/mod.rs`, narrow shared helpers in
`runtime/count_tokens.rs`. Deliver the interface/security contract above and
mock-only tests. It does not touch proxy scheduling, commands, or SQLite.

### Lane B — metric storage/report (independent first wave)

Boundary: migration 0024, `db.rs`, `repo/proxy_summary_metric.rs`, `repo/mod.rs`.
Deliver schema, terminal insert, plateau transaction, report, and privacy tests.
It does not touch runtime or CLI code.

### Lane C — spend control and CLI (depends on D's runtime control methods)

Boundary: `commands/proxy.rs`, `commands/cli.rs`, `router.rs`,
`bin/conclave-cli.rs`. Consume the exact config/status interface declared here;
do not invent defaults or restore state. Unit-test exact ON/OFF parsing,
validation atomicity, restart-OFF behavior, status redaction, and report mapping.

### Lane D — shadow orchestration (depends on A+B)

Boundary: `runtime/ctx_proxy.rs` plus integration tests. Add campaign state,
separate permit/cooldown/counters, SummaryJob, ordered pipeline, H0 projection,
runtime C authority, price math, and one-row terminal persistence. It must not
change `rewrite_body`, `upstream_body`, or response-stream plumbing.

### Integration order

Merge A and B in either order, then D, then C. Detoro reruns all header gates at
the integrated SHA. No implementer merges their own lane. If
`hybrid-summary-min-size-floor` changes H0 before D starts, D rebases on its
merged public interface and reruns ctxopt tests; it must not copy the old planner.

## Test plan and evidence

Beyond lane-local unit tests, D must run schema-aware end-to-end proxy tests:

1. Default/restart OFF: send eligible traffic; upstream receives original body
   byte-for-byte; generation mock hit count is zero; no summary row.
2. Checkpoint ON but summary OFF: M1 may count; generation hit count remains zero.
3. Invalid/missing price arm: command fails atomically; zero generation hits.
4. Armed happy path: original forward completes before the delayed generation
   mock; body remains byte-identical; exactly one generation call; A/B/C use
   schema-valid runtime-closed prefixes; one `measured` row with exact fixture
   math and all four usage buckets.
5. Disarm race before send: epoch mismatch persists `disarmed`, zero generation
   hits. Disarm after mock observes request may finish once; no retry/new job.
6. Every failure stage (A, tail boundary, plan, generation status/decode/nontext,
   projection shrink, B, C, pricing) persists exactly one bounded terminal row
   and never affects the forward.
7. Tool prompt-injection fixture containing literal forged delimiters stays one
   JSON envelope; no marker appears in DB/error/log fields.
8. Provider usage selects standard or long tier per request; threshold boundary
   tests cover `threshold-1`, `threshold`, `threshold+1` for forwarded A and
   generation total independently; hand-computed C_gen/n_h matches.
9. Semaphore/cooldown never waits in the forwarder; excess candidates increment
   drops and create no generation calls.
10. `summary-report` counts 30 rows/10 conversations/10 band rows correctly,
    includes a conversation with all misses, and never reports GO from averages.

Use targeted rustfmt on boundary files before the header gates; this checkout has
historically contained unrelated formatting drift, so do not mechanically
format unrelated files. There is no UI shot requirement.

## Acquisition procedure and binding GO bars

Building H1 stops with `summaryShadow=false`. The human separately supplies the
price schedule and arms a campaign. The operator records the returned campaign
id on the task ledger, verifies status shows the intended model/rates, then may
set the existing real-token ceiling for acquisition. Relaunch requires a new arm
and campaign id.

H1 may recommend GO-to-H2 only when one campaign (or explicitly combined
campaigns with identical model/prompt/price versions) has:

- at least 30 `measured` candidates;
- at least 10 distinct `conversation_hash` values;
- at least 10 candidates with real A in `[400_000, 500_000]`;
- combined count/generation failure rate ≤5%;
- at least 90% of measured candidates with B_h ≤ L;
- at least 80% with `n_h ≤ 2` using measured C_gen; and
- no conversation whose measured candidates all miss the applicable bars.

Report denominators and confidence limitations. Passing H1 authorizes only a
human decision about funding H2; it does not authorize H2 or apply.

For the failure bar, the denominator is
`measured + count_failure + generation_failure`; `below_ceiling`,
`no_candidate`, `projection_rejected`, `disarmed`, and queue drops are reported
separately and cannot dilute the rate. A pricing/metric failure prevents a row
from being `measured` and must be zero before GO.

### 400–500k data-availability dependency

M2 rarely observed organic conversations in the degradation band. H1 therefore
cannot promise a completion date or substitute 100k rows. The implementation
ships two honest outcomes:

1. **Organic acquisition:** the human arms and waits for real opted-in agent
   conversations to reach 400–500k. This requires no payload persistence but may
   never produce ten band samples.
2. **Controlled real-work campaign:** if organic data is insufficient, the human
   must separately authorize the tasks, duration, agents, and spend for long
   real conversations. Replaying repeated/synthetic filler is not acceptable for
   GO because it creates an artificially compressible distribution. Existing
   raw conversations are not available from the metric DB and must not be
   reconstructed from hashes.

Until one route yields the ten binding band samples, the verdict is
**INCONCLUSIVE / DATA DEPENDENCY**, not GO and not evidence that 100k behavior
generalizes. The implementation task must not launch a controlled campaign; that
is a later human spend/operations decision.

## Risk ledger

- **Spend after OFF:** a request already dispatched cannot be recalled. Epoch
  checks prevent queued sends; status exposes in-flight count; help text states
  the one in-flight limitation.
- **Message-cache reuse may be zero:** `tool_choice:none` and rewritten messages
  can invalidate message blocks. Preserved system/tools may still hit, but only
  actual usage decides. This can make H1 economics fail and is a valid result.
- **Count RPM pressure:** token-sized-tail search adds up to ten count calls plus
  A/B/C. The separate one-at-a-time sampler/cooldown bounds it; rate-limit errors
  become terminal rows, never retries.
- **Small selected results:** H0 correctly rejects when any tombstone/carrier
  does not shrink. Land `hybrid-summary-min-size-floor` before orchestration if
  Detoro accepts it; do not weaken strict shrink in runtime.
- **Model alias/pricing mismatch:** exact configured model gates spend. Price
  schedule is stored per row. Unknown/different models are skipped, never priced
  by analogy.
- **Long-context tier:** apply it independently to the forwarded request and the
  generation request from measured totals; one may be long while the other is
  standard.
- **Conversation hash privacy:** SHA-256 avoids raw text persistence but is still
  stable metadata. Expose it only in local SQL/report grouping, never CLI detail
  rows. If policy rejects this, replace with a campaign-keyed HMAC before build;
  do not fall back to raw first-message text.
- **App-global control:** H1 is shadow-only, so global arming cannot rewrite an
  agent, but it can spend on every matching opted-in conversation. Exact model,
  cooldown, OFF default, and human arming bound this risk. Per-agent apply
  isolation remains unresolved and out of scope.

## Rejected alternatives

- Reuse `proxy checkpoint on` as the spend arm: rejected because a previously
  safe/free measurement command must not silently begin paid generation.
- Restore ON/prices after restart: rejected because stale configuration would
  spend without a fresh human action.
- Pick a cheaper summarizer model: rejected by ruling `35f172a8`; it changes the
  hypothesis and may not be authorized by the captured credential.
- Omit tool definitions: rejected by ruling `0807943e`; retain their cache
  identity while `tool_choice:none` makes invocation impossible.
- Hardcode current prices: rejected by ruling `d631ccf5`; aliases and long tiers
  make stale price math silent and dangerous.
- Use H0 `count_prefix_end` or bytes/4 for C/tail/economics: rejected; runtime
  count closure/provider tokens are the authorities.
- Persist summaries for later review: rejected in H1 for privacy and scope. H2
  must define its own consented/ephemeral quality evidence path.
- Generate synthetic 400–500k filler for the GO denominator: rejected because
  it biases compressibility and does not represent real agent dependence.

## Escalation and completion

Implementation judgment inside these interfaces belongs to each implementer and
is recorded as task notes. Any change to the spend gate, price contract,
forwarded-byte invariant, model choice, H0/R9 boundary, closure authority,
privacy contract, H1 bars, or H2 exclusion is a task challenge to Detoro, final
ruler. Aoki is the design-of-record interpretation target; Detoro owns lane
creation, rulings, integration, and the human outcome report.

H1 implementation is complete only when all header gates pass on the integrated
SHA, status is demonstrably OFF after a fresh runtime, the mock suite proves zero
unarmed generation calls, and the task note explicitly says **no campaign was
armed**. The human decides whether and how to arm acquisition afterward.
