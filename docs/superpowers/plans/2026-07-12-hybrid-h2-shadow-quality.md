# Hybrid LLM Summary H2: Spend-Gated Shadow Quality Validation
<!-- conclave-plan:v1
{
"owner":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace","authority":"in-loop",
"planPath":"docs/superpowers/plans/2026-07-12-hybrid-h2-shadow-quality.md","baseSha":"83403eab7323433ce4b3d7c9522681c191887c16","escalation":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace",
"readingOrder":["docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md#114-quality-preservation-and-validation","docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md#116-ordered-live-measurement-plan-and-gonogo-bars","docs/superpowers/plans/2026-07-12-hybrid-h2-shadow-quality.md","docs/superpowers/plans/2026-07-12-hybrid-h1-shadow-economics.md","src-tauri/src/engine/runtime/ctx_proxy.rs#sample_summary","src-tauri/src/engine/repo/proxy_summary_metric.rs#SummaryReport"],
"boundary":["src-tauri/src/engine/runtime/quality.rs","src-tauri/src/engine/runtime/quality_fixtures.rs","src-tauri/src/engine/runtime/quality_audit.rs","src-tauri/src/engine/runtime/ctx_proxy.rs","src-tauri/src/engine/runtime/mod.rs","src-tauri/src/engine/repo/proxy_quality_metric.rs","src-tauri/src/engine/repo/proxy_summary_metric.rs","src-tauri/src/engine/repo/mod.rs","src-tauri/src/engine/migrations/0025_proxy_quality_metric.sql","src-tauri/src/engine/db.rs","src-tauri/src/engine/commands/proxy.rs","src-tauri/src/engine/commands/cli.rs","src-tauri/src/engine/router.rs","src-tauri/src/bin/conclave-cli.rs","src-tauri/src/engine/runtime/quality_fixtures/*.json","src-tauri/Cargo.toml"],
"consumes":["src-tauri/src/engine/runtime/ctx_proxy.rs#sample_summary","src-tauri/src/engine/runtime/summary.rs#GeneratedSummary","src-tauri/crates/ctxopt/src/summary.rs#SummaryPlan","src-tauri/crates/ctxopt/src/summary.rs#SummaryProjection","src-tauri/src/engine/repo/proxy_summary_metric.rs#SummaryReport","src-tauri/src/engine/runtime/count_tokens.rs#CountCredential"],
"produces":["src-tauri/src/engine/runtime/quality.rs#evaluate_quality_case","src-tauri/src/engine/runtime/ctx_proxy.rs#QualityCandidate","src-tauri/src/engine/repo/proxy_quality_metric.rs#QualityReport","proxy.qualityShadow","proxy.qualityFixtures","proxy.qualityReport","proxy.qualityAudit"],"gates":["cargo test --manifest-path src-tauri/crates/ctxopt/Cargo.toml","cd src-tauri && cargo test engine::runtime::quality","cd src-tauri && cargo test engine::runtime::quality_fixtures","cd src-tauri && cargo test engine::runtime::quality_audit","cd src-tauri && cargo test engine::repo::proxy_quality_metric","cd src-tauri && cargo test engine::commands::proxy","cd src-tauri && cargo test --lib","cd src-tauri && cargo clippy --all-targets -- -D warnings","git diff --check"]
} -->

## Goal and authorization boundary

Implement H2 **shadow quality validation only**. For an in-memory summary
candidate, measure two properties without changing the real upstream request:

1. faithfulness: every summary claim is grounded in cited source tool results,
   critical information is not omitted, and a source-only probe set remains
   answerable from the summary; and
2. behavioral non-inferiority: the task model's no-tools next-action plan from
   projected context is not materially worse than its plan from original
   context under a blind evaluator.

Human authorization on 2026-07-12 funds building H2. It does not authorize
model calls merely because the app was rebuilt. H2 has a separate in-memory arm
and hard case budget. H1 being armed never arms H2. Restart returns H2 to OFF.

H2 produces a report for a later human decision. Passing H2 does not authorize
H3 design, apply, forwarded-byte rewriting, or a live agent trial.

## Prominent input-coupling precondition

There are two deliberately different paths:

- **Build/test path:** deterministic synthetic fixtures call pure builders and
  schema-aware mock upstreams directly. H1 and H2 remain OFF, no credential is
  needed, and CI makes zero real model calls.
- **Live GO path:** H2 consumes raw in-memory artifacts at the end of a successful
  H1 `sample_summary`. The same H1 campaign must still be armed and must satisfy
  every H1 economics bar. H2 must separately be armed against that exact H1
  campaign. Every live admission rechecks both gates before reserving budget.

An armed-but-inconclusive H1 campaign is **not** sufficient. The dependency is
mechanical, not an operator checklist: `H1Gate::Pass` is computed from the landed
`SummaryReport`. If H1 later falls below a bar, H2 stays configured but performs
zero further model calls until H1 is again Pass or the human disarms it.

## Design-of-record and binding rulings

- Spec authority: §11.4, §11.6 H2, and R9 in
  `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md`.
- H1 candidate authority: the original request, H0 `SummaryPlan`, generated
  summary, strict `SummaryProjection`, source envelope, captured upstream/
  `CountCredential`, and hashes already alive inside `sample_summary`. H2 does
  not reconstruct raw inputs from SQLite; H1 deliberately persisted none.
- Ruling `49a5d987` / challenge `940295fe`: every live H2 case requires the same
  still-armed H1 campaign and `H1Gate::Pass`; fixtures remain independently
  buildable/testable.
- Ruling `467ecb7a` / challenge `2f35f7a2`: one pinned evaluator model performs
  source-only probe generation, faithfulness verification, and blind judging;
  it must differ from the H1/task/summarizer model. Behavioral A/B replays use
  the task model. The separate H2 arm has a hard `maxCases` budget decremented
  before scheduling. Evaluator authentication is preflighted fail-closed with
  the captured credential before a campaign runs quality calls.
- Ruling `a62412e0` / challenge `043acb1a`: GO requires at least 100 successful
  cases: at least 30 live H1 candidates and at least 70 fixture evaluations;
  each required stratum has at least 10 cases. Human audit uses 12 synthetic
  fixture cases (4 accepted, 4 rejected, 4 near-threshold), never raw live data.

## Non-goals

- No apply path, summary reuse, checkpoint mutation, H3 trial, or forwarded-body
  change.
- No H1 generation/economics redesign and no weakening of its spend arm.
- No evaluator-model auto-selection, fallback, or change after H2 arm.
- No evaluator call by the task/summarizer model; the summarizer never grades
  itself.
- No prose-similarity score. Behavior is correctness, constraint adherence, and
  selected next action.
- No persistence of original/projected requests, source results, summaries,
  probes, verifier prompts/responses, replay plans, judge prompts/responses,
  credentials, or raw upstream errors.
- No live fixture evaluation without a qualifying H1 carrier request supplying
  the captured upstream and credential. Credentials never enter CLI arguments,
  config, status, SQLite, or a long-lived campaign object.
- No real model call in tests/CI, including ignored tests run by the default
  suite.

## Global constraints (every implementation lane inherits these)

1. **Shadow invariant:** H2 is called only after H1 has already forwarded the
   original request and produced a projection. It returns no body to
   `forward_inner` and cannot influence the real response or agent state.
2. **Double-spend gate:** a quality call requires (a) same-campaign
   `H1Gate::Pass`, (b) an armed H2 epoch targeting that H1 campaign, (c) a free
   H2 permit/cooldown, and (d) an atomically reserved remaining case. H1 ON alone
   is never enough.
3. **Hard budget:** one case reservation permits at most five quality calls:
   source-probe generation, faithfulness verification, original replay,
   projected replay, and blind judge. One campaign permits one additional
   evaluator-auth preflight call. Thus absolute maximum calls are
   `1 + 5*maxCases`. Failures consume the reserved case; no retry or refund.
4. **Off-path:** use a separate one-permit semaphore and `try_acquire_owned`.
   Never wait in the H1 sampler or forwarder. Quality work runs in a spawned task
   after the measured H1 row persists successfully.
5. **Credential containment:** each job carries the captured H1 upstream and
   `CountCredential`, derives no `Debug`/`Serialize`/`Deserialize`, uses a
   no-redirect client with fixed timeouts/no retries, and drops credentials at
   job end. Error labels are fixed allowlists.
6. **Evaluator independence:** `evaluator_model != task_model` is validated at
   arm and admission. Probe/verifier/judge prompt and rubric versions are pinned
   code constants and persisted. The arm must name the exact compiled rubric
   version; any mismatch fails closed.
7. **Blindness:** replay call order and judge labels are randomized per case.
   The judge receives opaque A/B plans and never the original/projected label.
   Runtime maps the bounded verdict back only after parsing.
8. **Privacy:** raw live values exist in one `QualityJob` only. Synthetic fixture
   outputs may additionally enter the bounded in-memory human-audit reservoir;
   they never enter SQLite/files/logs. SQLite stores counts, bounded enums,
   booleans, model/prompt versions, usage totals, and SHA-256 hashes. CLI reports
   never contain raw data.
9. **One terminal row:** each reserved case persists one outcome. Missing values
   are NULL, never zero. A repo API accepting only typed outcome/stage enums is
   the sole write seam.
10. **No CI spend:** all network tests use local mock upstreams and assert the
    exact path/model/hit count. There is no environment-variable escape hatch to
    a real provider in test code.
11. **No UI product surface:** the only transient human-audit page is served
    from backend memory on loopback with `Cache-Control: no-store`; no `src/` UI
    file changes, so the UI pixel gate is not applicable.

## Quality rubric and case contract

Pin these constants in `runtime/quality.rs`:

```rust
pub const QUALITY_METHOD_VERSION: &str = "h2-shadow-quality-v1";
pub const QUALITY_RUBRIC_VERSION: &str = "hybrid-quality-rubric-v1";
pub const PROBE_PROMPT_VERSION: &str = "source-probes-v1";
pub const FAITHFULNESS_PROMPT_VERSION: &str = "faithfulness-v1";
pub const REPLAY_PROMPT_VERSION: &str = "next-action-v1";
pub const JUDGE_PROMPT_VERSION: &str = "blind-next-action-v1";
pub const MAX_PROBES: usize = 32;
pub const MAX_CALLS_PER_CASE: u64 = 5;
pub const QUALITY_CARRIER_COOLDOWN_MS: u64 = 60_000;
pub const MAX_FIXTURES_PER_CARRIER: usize = 3;
```

The seven persisted stratum tags are:

`side_effecting_output | long_log | exact_error | rejected_alternative |
parallel_tool_cycle | prompt_like_tool_text | mutation_or_open_work`.

Fixture tags come from the validated manifest. Live tags are the union of a
deterministic structural classifier (tool names, parallel-cycle shape, result
size, `is_error`, literal prompt-like markers) and validated source-only probe
categories. The evaluator cannot clear a deterministic tag. For semantic tags
such as rejected alternatives/open work, at least one cited probe is required
before the tag is accepted.

A critical fact is one whose loss or fabrication can change: the user goal,
constraint, or acceptance criterion; whether a mutation/side effect occurred;
the next safe action; an unresolved blocker; the conclusion of negative
evidence; or an exact identifier/path/command/version/error/number required to
continue or diagnose. Style, ordering, and non-load-bearing wording are
noncritical.

Use an in-memory-only case type:

```rust
struct QualityCandidate {
    source_kind: QualitySource, // Live or Fixture{id,tags}
    h1_campaign_id: String,
    conversation_hash: String,
    checkpoint_id: String,
    source_boundary_hash: String,
    summary_hash: String,
    task_model: String,
    summarizer_response_model: String,
    summary_prompt_version: String,
    upstream: String,
    credential: CountCredential,
    original_request: Value,
    projected_messages: Value,
    summary: String,
    source_envelope: String,
    selected_tool_use_ids: Vec<String>,
}
```

The type must not implement debug/serialization. Validate the H0 projection
again before any quality call; a failure persists `structural_failure` and is a
hard H2 NO-GO signal.

## H1 gate and candidate handoff

Add a pure `H1Gate::{Pass, Inconclusive, Fail}` helper over a campaign-scoped
H1 report with no time cutoff. Extend `proxy_summary_metric` with
`report_campaign(campaign_id)` rather than approximating with a 24-hour report.

- `Inconclusive`: fewer than 30 measured rows, 10 conversations, or 10 real-A
  rows in `[400_000,500_000]`.
- `Fail`: minimum data exists and any economics bar fails: failure rate >5%,
  low-water pass <90%, two-turn pass <80%, any all-miss conversation, or any
  metric-invalid row. A structural/projection failure is separately visible and
  cannot be treated as an economics pass.
- `Pass`: minimum data exists and every bar above passes.

Modify the end of `sample_summary` narrowly:

1. Keep generated summary, plan, projection, source envelope, original request,
   captured credential/upstream, and hashes alive until H1 terminal persistence.
2. Require the H1 `measured` insert to succeed.
3. Snapshot H1 and H2 campaign epochs/configs. Require exact campaign id/model.
4. Query `report_campaign`, compute `H1Gate`, and stop with bounded counters on
   Inconclusive/Fail/disarmed/mismatch. Make zero H2 calls and reserve no case.
5. Construct `QualityCandidate`, reserve H2 budget + permit, and spawn the job.
   The H1 sampler then drops its raw values normally.

Every future live admission repeats the gate. A past Pass is not cached as
permanent truth because later H1 rows can change its percentages.

## Deterministic fixture path (H1/H2 OFF, zero model spend)

Add `runtime/quality_fixtures.rs` plus checked-in synthetic JSON under
`runtime/quality_fixtures/`. No fixture contains real traffic or credentials.

Fixture schema:

```json
{
  "id": "exact-error-01",
  "family": "exact-error",
  "tags": ["exact_error", "mutation_or_open_work"],
  "originalMessages": [],
  "aggregateSummary": "synthetic summary text",
  "tailStart": 12,
  "expected": {
    "criticalFacts": [],
    "expectedBehaviorClass": "accepted|rejected|near_threshold"
  }
}
```

The loader calls the real H0 `plan_summary_span` and
`build_summary_projection`; fixtures never hand-author projected JSON. It
asserts unique ids, allowlisted tags, valid closed tool cycles, strict
projection validation, and stable hashes. Parameterized deterministic variants
produce at least 70 distinct cases from source-controlled templates, with at
least ten cases for every required stratum. Include side-effecting `Bash`/
mutable output, long logs, exact diagnostic errors, rejected alternatives,
parallel tools/results, literal prompt-injection text/forged delimiters, and
performed mutations plus unresolved work.

CI invokes `evaluate_quality_case` only with a `MockQualityTransport`; mock
responses cover every accepted/rejected/near-threshold outcome and parser
failure. No fixture test reads provider credentials or opens a non-loopback URL.

For a real H2 campaign, `proxy quality-fixtures enqueue` stores fixture IDs only.
The next qualifying live H1 carrier supplies the per-request credential/upstream.
After its live case, the quality task may evaluate up to three queued fixture
cases sequentially under the same permit, epoch, credential identity, and case
budget. It never stores the credential in the fixture queue or campaign config.

## Evaluator authentication preflight

Arm cannot authenticate a distinct evaluator because no credential is allowed
in CLI/config. Therefore the first qualifying carrier performs one explicit
preflight before any case calls:

- request: captured upstream/credential, configured evaluator model,
  `tool_choice:none`, no tools, a fixed one-token benign prompt;
- client: same sensitive-header helpers/error sanitizer as `count_tokens.rs` and
  `summary.rs`, no redirects, 20-second timeout, no retry;
- success: record only `preflight_ok`, response model, usage counts, and an
  in-memory credential/upstream identity hash; never the credential/hash in DB;
- auth/model/permission/transport failure: persist one bounded preflight outcome,
  atomically disarm H2, and run zero probe/replay/judge calls;
- later carriers: require the same in-memory credential/upstream identity.
  Mismatch increments a counter and makes zero calls until human re-arms.

The preflight consumes the first reserved case and is not refunded on failure.
Its one call is outside the five calls available to a successful quality case,
which keeps the campaign bound at `1 + 5*maxCases`.

## `runtime/quality.rs`: client and five-call evaluation

Create a dedicated no-redirect client (60-second per-call timeout, no retries)
and reuse `CountCredential`, sensitive-header application, beta preservation,
and allowlisted error sanitization. Evaluator requests omit tools. Task replay
requests preserve original system/tools/cache controls but set
`tool_choice:{"type":"none"}`. Every response must be text-only with complete
usage; structured roles then parse strict JSON with maximum lengths/counts.

### Call 1: source-only probe generation

The evaluator sees only the JSON source envelope and pinned criticality rubric,
never the summary. Require 4–32 unique probes. Each probe contains an opaque id,
one allowlisted category, question, expected answer, cited `tool_use_id` values,
and critical boolean. Reject missing citations, unknown source ids, duplicate ids,
oversized fields, or fewer than one probe for every category present in the
source. The raw probe set remains in memory; persist its hash/count/categories.

### Call 2: faithfulness verification

The evaluator sees source envelope, candidate summary, and probes. It must:

- split the summary into atomic claims and check **every** claim against cited
  source tool ids;
- report total/supported/unsupported claim counts and whether each unsupported
  claim is critical;
- identify critical/noncritical omissions relative to source/probes; and
- mark every probe `retained=true|false` based on whether its expected fact is
  recoverable from the summary.

Parser invariants reconcile totals and probe ids exactly. Define per-case probe
recall as `retained_probes / total_probes`. A critical hallucination is a
critical unsupported claim; a critical omission is a critical omitted fact or
critical unretained probe. Persist counts/recall/hashes only.

### Calls 3 and 4: original/projected next-action replay

Randomize which context is called first. Both calls use `task_model`, identical
system/tools, `tool_choice:none`, same max output, and the same pinned instruction:
produce one bounded next-action plan without executing tools. The only varying
input is original versus projected messages. Require strict JSON fields for
selected next action, constraints observed, expected evidence/result, and
blocked/ready state. Raw plans remain in memory; persist hashes and usage only.

### Call 5: blind judge

Randomly label the two plans A/B independently of call order. Build the judge
request from the full original in-memory context (original system, tools, and
messages), replace only `model=evaluator_model` and `tool_choice:none`, then
append the source-derived probes/expected answers, rubric, and opaque A/B plans.
This gives the judge the actual constraints/evidence rather than a second lossy
reference summary. It is not told which context produced either plan. It scores
each plan separately on three booleans:

- `correct`: action follows the evidence/current state;
- `constraint_adherent`: no user/spec/safety constraint is violated; and
- `next_action_match`: selected action is appropriate for the unresolved work.

Overall pass is all three true. Parse A/B, map back in memory, and persist
`original_pass`, `projected_pass`, six dimension booleans, and bounded
`original_win|tie|projected_win|both_fail`. Persist no rationale or plan text.

Any call/schema failure terminates the case with one typed stage, consumes the
case, performs no remaining calls, and never retries.

## Behavioral statistic

For each successful case let
`d_i = projected_pass_i - original_pass_i`, in `{-1,0,1}`. The point estimate is
the mean difference in percentage points. Compute a deterministic, stratified
cluster bootstrap with 10,000 replicates:

- live cluster = `conversation_hash`;
- fixture cluster = fixture family;
- resample live clusters and fixture clusters separately with replacement,
  preserving each source's observed case count;
- seed `ChaCha20Rng` from SHA-256 of
  `quality_campaign_id || rubric_version || "paired-bootstrap-v1"`; and
- use the 2.5th and 97.5th percentile replicate differences as the 95% CI.

Persist the method version and seed hash, not raw samples. Require at least two
live conversation clusters and two fixture families; otherwise the report is
insufficient regardless of case count. Add `rand_chacha`/`rand_core` explicitly
to `src-tauri/Cargo.toml`. Unit tests use fixed case vectors with pinned expected
point/CI outputs so dependency upgrades cannot silently move the gate.

H2 passes behavioral non-inferiority only when the 95% CI lower bound is
`>= -0.05` (no worse than minus five percentage points).

## Spend gate and exact CLI

Add separate IPC `proxy.qualityShadow`; it never calls or aliases
`proxy.summaryShadow`.

OFF:

```text
conclave proxy quality-shadow off
```

ON (every flag required):

```text
conclave proxy quality-shadow on \
  --h1-campaign-id <currently-armed-passing-h1-id> \
  --evaluator-model <exact-model-different-from-task-model> \
  --rubric-version hybrid-quality-rubric-v1 \
  --max-cases <positive-integer>
```

The async command handler first verifies the named H1 campaign is currently
armed, model-identical to its rows, and `H1Gate::Pass`. It rejects evaluator
equal to H1/task model, rubric mismatch, zero/out-of-range `maxCases` (cap 1,000),
or any missing/duplicate/unknown flag. Validation failure leaves prior H2 state
unchanged. Success mints a quality campaign id, bumps epoch, stores config with
`remainingCases=maxCases`, and marks evaluator preflight pending. Restart is
OFF; nothing restores it.

Budget reservation and epoch snapshot occur under one lock before spawn. The
counter never underflows. OFF increments epoch then clears config/fixture queue/
audit reservoir. A request already on the wire cannot be recalled; status shows
in-flight and help states this limitation.

Additional commands:

```text
conclave proxy quality-fixtures enqueue --manifest h2-adversarial-v1
conclave proxy quality-report [--since-hours N] [--campaign-id ID]
conclave proxy quality-audit start --campaign-id ID
conclave proxy quality-audit stop
```

Fixture enqueue performs no model call and stores ids only. Report is read-only.
Audit start performs no model call; it starts the bounded synthetic-only review
reservoir/page described below.

`proxy status` adds only bounded state: armed, quality/h1 campaign ids, evaluator
model, versions, max/remaining cases, preflight state, fixture queue length,
dropped/H1-blocked/model/credential-mismatch counters, audit progress, and
in-flight count. It never exposes credentials or raw case data.

## Human audit without raw persistence

The audit is synthetic-fixture-only. Live cases can never enter the audit queue.
`quality_audit.rs` starts an ephemeral Axum server on `127.0.0.1:0` with a random
one-use URL token. It holds at most 12 completed fixture bundles in memory:
source-controlled fixture, generated probes, synthetic summary, both plans, and
bounded verifier/judge output. It writes no file or DB raw field.

Responses set `Cache-Control: no-store`, strict local CSP, no external assets,
and no referrer. The server expires after two hours or 12 submitted reviews,
then drops all bundles and token. The page reveals original/projected labels only
after the human records an initial judgment, preventing hindsight bias. Persist
only case id, audit bucket, `agree|disagree`, rubric version, and timestamp in a
small audit table; no comments/free text.

Selection is deterministic and stratified from fixture results: four automated
accepts, four rejects, four cases nearest the probe/behavior thresholds. The 12
collectively cover all seven tags. H2 cannot GO with fewer than 12 completed
audits or any unresolved disagreement; a disagreement requires a new rubric/
prompt version and a new campaign, not an edited row.

This loopback page is backend-generated and touches no `src/` UI. Tests inspect
headers/token expiry/live-case rejection with local requests; no screenshot gate.

## Persistence: migration 0025 and repository

Create `proxy_quality_metric` plus `proxy_quality_audit`, register user_version
25, and add `repo::proxy_quality_metric`. Raw-shaped text columns are forbidden.

Metric columns:

- identity: `created_at`, `quality_campaign_id`, `h1_campaign_id`, `case_id`,
  `source_kind`, `fixture_id`, `fixture_family`, seven tag booleans,
  `conversation_hash`, `checkpoint_id`, source/summary/probe/original-plan/
  projected-plan/judge hashes;
- versions/models: task, summarizer response, evaluator response models;
  quality/rubric/probe/faithfulness/replay/judge versions;
- structure/faithfulness: `structural_pass`, claims total/supported/unsupported,
  critical/noncritical hallucinations, critical/noncritical omissions, probes
  total/retained, probe recall;
- behavior: original/projected dimension booleans, overall passes, bounded
  comparison label;
- usage: input/cache-creation/cache-read/output counts separately for preflight,
  probe, faithfulness, original replay, projected replay, and judge;
- terminal: typed outcome, failure stage, allowlisted upstream error type.

Use NULL for stages not reached. Hash/id fields have lowercase-hex/UUID shape
validation before SQL plus CHECK constraints. Labels/tags/stages/models/versions
have length caps; models/versions must match armed constants/config. The sole
`insert_terminal` API accepts typed enums and no `String` payload for failures.
Tests inject secrets, prompts, forged delimiters, summaries, probe questions,
plans, and hostile upstream messages and assert none appears in any table/log.

Audit rows reference fixture quality rows and carry only bounded verdict fields.
No update can change an existing audit verdict; rerun means a new campaign.

## `QualityReport` and binding GO predicate

Report campaign-scoped denominators and failures, never averages alone:

- reserved/terminal/successful cases, live/fixture split, distinct live
  conversations, fixture-family counts, seven tag counts;
- call/schema/structural failures by stage and usage totals by role/model;
- critical/noncritical hallucination and omission totals;
- probes total/retained and aggregate recall (also min/per-source distributions);
- original/projected dimension and overall pass rates, paired point estimate,
  bootstrap CI/method version, win/tie/both-fail counts;
- 12-case audit bucket/verdict counts and unresolved disagreements.

H2 GO requires all of the following in one compatible campaign/version set:

1. at least 100 successful cases;
2. at least 30 live cases and 70 fixture cases;
3. at least 10 cases in each of the seven required strata;
4. zero structural failures;
5. zero critical hallucinations;
6. zero critical omissions;
7. aggregate source-probe recall `>= 0.98` (report live and fixture recall too;
   neither source may be below 0.98);
8. behavioral paired 95% CI lower bound `>= -0.05`;
9. at least two live conversation clusters and two fixture families for CI;
10. all 12 synthetic audits complete (4/4/4, all tags) with zero unresolved
    disagreements; and
11. the linked H1 campaign remains armed and `H1Gate::Pass` at report time.

Failed model/schema cases do not count toward 100 and are reported explicitly.
No failure can be averaged away into recall. Passing authorizes only a human
decision about H3 apply-path design.

## Implementation lanes and integration order

Detoro creates/owns downstream tasks. No implementer merges their own lane.

### Lane A: deterministic fixtures + pure rubric/statistics (first wave)

Boundary: `runtime/quality_fixtures.rs`, fixture JSON, pure quality types/
bootstrap helpers in `runtime/quality.rs`, Cargo dependencies, **and the
`pub mod quality; pub mod quality_fixtures;` declarations in `runtime/mod.rs`**
(mark `#[allow(dead_code)]` until consumers wire them; Lane B extends `mod.rs`
afterward). Deliver loader, 70-case manifest, tag/critical rubric, mock transport
(the transport trait + `MockQualityTransport` + `evaluate_quality_case` shape are
Lane A's deliverable — Lane B implements the real network transport against the
merged trait), and pinned CI tests. No network client, proxy, DB, or commands.

> **Boundary guard (Detoro ruling 2026-07-12, defect found by Dabin, challenge
> a5cb41f4):** any lane that creates a NEW `runtime/` module must own the parent
> `runtime/mod.rs` `pub mod` declaration for it — a produced module file is inert
> and its `cargo test engine::runtime::<mod>` gate cannot compile without it. The
> v1 header's full boundary already lists `runtime/mod.rs`; this was a per-lane
> prose mis-split (H1 Lane A correctly included it). Lane A owns the two
> declarations; Lane B, sequenced after A, inherits the merged `mod.rs`.
>
> **Lockfile corollary (Detoro ruling 2026-07-12, challenge bac46765, found by
> Dabin):** any lane whose boundary includes `Cargo.toml` implicitly includes
> `src-tauri/Cargo.lock` for the mechanical delta cargo generates from that
> manifest edit — and nothing else. Reverting the lock delta makes
> `cargo --locked` reject the manifest as stale. Lane A commits the two-line
> root-package dependency entries for rand_chacha/rand_core.

### Lane B: evaluator/replay client (first wave, independent)

Boundary: network portion of `runtime/quality.rs`, further `runtime/mod.rs`
wiring beyond Lane A's two `pub mod` declarations, narrow reuse helpers in
`count_tokens.rs` if required. Sequenced after Lane A merges (shared
`quality.rs`); build against Dabin's merged transport trait +
`evaluate_quality_case` signature as-is. Publish the exact evaluator error-kind
set in the READY note so Lane C can define its DB `error_type` allowlist against
real names, not plan prose. Deliver preflight + five role calls,
strict schemas, fixed errors, usage capture, credential containment, and mock
tests. Read the actual merged error-kind vocabulary before Lane C defines DB
allowlists.

### Lane C: quality repository/report (first wave, independent after A types)

Boundary: migration 0025, `db.rs`, `repo/proxy_quality_metric.rs`,
`repo/proxy_summary_metric.rs#report_campaign`, `repo/mod.rs`. Deliver typed
terminal insert, privacy/shape constraints, H1 gate report, audits, report, and
GO predicate.

### Lane D: H1-to-H2 orchestration (depends A+B+C)

Boundary: `runtime/ctx_proxy.rs`. Add separate campaign/epoch/budget/permit,
candidate handoff after successful H1 insert, live H1 recheck, preflight,
fixture queue/carrier loop, and one terminal row per reserved case. Do not change
forwarded-body/rewrite paths or H1 economics math.

### Lane E: CLI + ephemeral audit (depends C+D)

Boundary: `runtime/quality_audit.rs`, `commands/proxy.rs`, `commands/cli.rs`,
`router.rs`, `bin/conclave-cli.rs`. Add exact commands/status, audit loopback
server, and mock-only tests. No `src/` UI.

Merge A/B in either order, then C, then D, then E. After each merge, consumers
must verify actual merged enum/error names rather than the plan's prose. Detoro
reruns every header gate at the integrated SHA.

## Test plan

1. Fresh runtime: H1/H2 OFF, empty budgets; eligible traffic causes zero H2
   calls/rows. H1 ON alone still causes zero H2 calls.
2. Fixture loader: all 70+ cases deterministic, unique, structurally valid,
   tag minima satisfied; real H0 projection used; injection strings remain JSON
   values.
3. H1 gate table: every threshold boundary, all-miss conversation, invalid row,
   Inconclusive/Fail/Pass; arm rejects non-Pass/wrong/disarmed campaign.
4. H2 arm: separate epoch, exact rubric, evaluator != task model, max-case bounds,
   atomic validation, restart OFF, disarm race before each of five calls.
5. Budget: decrement before spawn, no underflow/refund, at most
   `1+5*maxCases` mock hits under concurrency/failures.
6. Preflight: distinct evaluator model/body, captured upstream/credential,
   success identity, auth/model/permission/redirect/timeout failure auto-disarm,
   credential mismatch zero calls.
7. Probe call: summary absent from request, 4–32 probes, citation/category/id/
   count reconciliation, every malformed response fails closed.
8. Faithfulness: every claim/probe reconciled; critical hallucination/omission,
   negative evidence, exact identifiers/errors/numbers, and unknown-not-infer
   fixtures; no raw values persisted.
9. Replay: byte-identical setup except messages, task model, tools disabled,
   randomized call order, no side effects.
10. Judge: evaluator model, A/B blindness/randomization, dimension mapping,
    response order cannot reveal original/projected label.
11. Bootstrap: fixed vectors, cluster/source stratification, deterministic seed,
    percentile edges, insufficient clusters, `-0.05` boundary.
12. Live handoff E2E: H1 measured insert precedes quality scheduling; original
    forward remains byte-identical; raw candidate dropped after row; H1 later
    failing blocks subsequent calls.
13. Fixture queue: ids only, up to three per qualifying carrier, budget/epoch/
    credential rechecks between cases, no credential persistence.
14. Metrics: one row per reserved case at every failure stage; SQL CHECKs reject
    raw/unknown values; report enforces every GO denominator and cannot hide
    live/fixture/tag failures.
15. Audit: live cases rejected, 12-case 4/4/4/tag selection, no-store/CSP/token/
    expiry, one-shot verdicts, raw queue zeroized/dropped on stop/disarm/timeout.
16. Full test/clippy gates use only loopback mocks; scan test source for any real
    provider host/credential escape hatch.

No UI shots are required because no `src/` UI is touched.

## Live acquisition and operational order

1. Build/restart; verify both `summaryShadow=false` and `qualityShadow=false`.
2. Human separately arms/runs H1. Do not arm H2 while H1 is Inconclusive/Fail.
3. When the named H1 campaign passes, human chooses H2 evaluator model and
   `maxCases`, then runs the exact H2 arm command. This is the H2 spend action.
4. First qualifying carrier preflights evaluator authorization. Failure disarms
   and returns to the human; no fallback model.
5. Enqueue `h2-adversarial-v1` fixtures and start the audit page before fixture
   evaluation if the 12-case human audit is intended in that run.
6. Continue real work until at least 30 post-Pass live H2 candidates and 70
   fixture cases complete. H1 must remain armed/Pass throughout.
7. Run campaign-scoped report. An unmet input/stratum/audit/CI bar is
   INCONCLUSIVE or NO-GO as labeled; it is never silently pooled away.
8. Human reviews outcome and alone decides whether to fund H3 design.

## Risk ledger

- **Long dependency chain:** H2 live data cannot begin until H1 passes, remains
  armed, and continues producing new ephemeral candidates. This may take a long
  time; fixtures validate machinery but cannot replace 30 live cases.
- **Evaluator authorization:** distinct model access is not implied by task-model
  access. First-carrier preflight is binding; no fallback or same-model grading.
- **Spend amplification:** successful case = five calls and a carrier may run up
  to four cases. `maxCases`, one permit, cooldown, call-by-call epoch checks, and
  no retry are load-bearing.
- **Judge correlation:** one evaluator model generates probes, verifies, and
  judges. Separate prompts/calls plus human fixture audit reduce but do not erase
  correlated bias. A disagreement requires versioned redesign, not hand-editing.
- **Baseline can be wrong:** non-inferiority compares projected to original task
  behavior. Report absolute original/projected pass rates and both-fail counts;
  do not describe a tie of two failures as good quality.
- **Bootstrap dependence:** repeated cases in one conversation are correlated;
  cluster bootstrap and minimum clusters prevent false precision. Report method
  and source split.
- **Probe-generator omission:** source-only generation avoids summary bias but can
  miss a source fact. Critical fixture expectations and human audit are the
  guard; no automated recall number alone proves completeness.
- **Raw-memory lifetime:** audit keeps synthetic raw outputs up to two hours;
  live raw data is never queued. Disarm/stop/timeout/drop paths must clear all
  buffers.
- **App-global scope:** H2 is shadow-only but can spend on matching conversations.
  exact H1 campaign/model, credential identity, cooldown, and hard case budget
  contain it. Per-agent H3 isolation remains unresolved.
- **Metric gaming by failures:** only successful cases enter GO denominators;
  failure counts remain explicit. Structural or critical failures are hard stops.

## Rejected alternatives

- Let H2 run while H1 is armed but inconclusive: rejected by ruling `49a5d987`;
  spec orders H1 PASS before H2 live spend.
- Use the task/summarizer model as verifier: rejected by ruling `467ecb7a`;
  summarizer cannot grade itself.
- Boolean H2 arm with no case cap: rejected; five calls/case makes spend
  unbounded.
- Put evaluator credential/model key in CLI/config: rejected; capture the
  existing request credential, preflight distinct-model access, and never
  persist it.
- Reconstruct candidates from H1 hashes/metrics: impossible by privacy design;
  raw summaries/sources were never stored.
- Generate probes after seeing the summary: rejected because it can select only
  retained facts and inflate recall.
- Judge plan similarity: rejected; identical prose can choose the wrong action.
- Persist raw cases for later audit: rejected. Only synthetic fixtures enter a
  bounded no-store in-memory page; live raw never leaves the job.
- Count 100 fixture cases and token live coverage: rejected by ruling
  `a62412e0`; >=30 live and >=70 fixtures are binding.
- Use ordinary independent-proportion CI: rejected because original/projected
  plans are paired and live cases cluster by conversation.
- Allow H2 PASS to begin H3 automatically: rejected; human authorization is a
  separate decision.

## Escalation and completion

Implementation judgment within fixed interfaces belongs to each implementer and
is recorded on its task. Any change to H1 Pass coupling, separate arm/budget,
model independence, preflight, five-call maximum, privacy, fixture/live/tag
minimums, criticality rubric, CI method/bar, audit policy, or H3 exclusion is a
task challenge to Detoro, final ruler. Aoki is the design-of-record
interpretation target. Detoro owns lane creation, integration, and human report.

H2 build completion requires all header gates green on the integrated SHA,
fresh-runtime status proving H2 OFF, mock hit counts proving no unarmed/CI model
calls, fixture/tag manifests validated, and a READY note explicitly stating that
no live H2 campaign was armed. The human's later arm is the first authorized H2
spend action.
