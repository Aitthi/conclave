// Data model + fixtures for the ctx-proxy checkpoint-measurement dashboard.
//
// Mirrors the SQLite table `proxy_checkpoint_metric`. One row = one sampled
// /v1/messages request the proxy evaluated for an infinity-turn checkpoint.
//
// Economics (spec 2026-07-11-infinity-turn-checkpoint-design.md §2):
//   q = s_net / r          recoverability ratio, the headline metric.
//   break-even turns  n = 11.5 / q - 12.5
//   n <= 2  requires  q >= 0.793   -> the GO bar.
// Decision: GO iff avg(q) >= GO_THRESHOLD; below that leans NO-GO.
//
// Fixtures below are ILLUSTRATIVE, anchored to a REAL early window (Detoro,
// 2026-07-12, ceiling=100k, organic load): 12 samples, count_failure=0
// throughout, q 0.164..0.44 (avg 0.256), break-even 13.6..57.8 turns, every
// outcome "saturated" vs the 100k ceiling. The live measurement has since grown
// to 25 samples with q up to ~0.526 — still well under the 0.793 GO line, so the
// verdict is unchanged; swap Q_SERIES / GROSS_CANDIDATE here when wiring real
// data. Numbers are plausible, not invented — they reproduce the recorded
// aggregates. Nothing calls Date.now(): timestamps are frozen literals so the
// canvas renders deterministically.

export const GO_THRESHOLD = 0.793; // q at which break-even hits ~2 turns
export const CEILING_TOKENS = 100_000;
export const TARGET_BREAK_EVEN_TURNS = 2;

export type Outcome = "below_ceiling" | "eligible" | "saturated";

export interface CheckpointSample {
  id: number;
  createdAt: string; // ISO, frozen literal
  model: string;
  q: number;
  outcome: Outcome;
  countFailure: 0 | 1;
  grossCandidateTokens: number; // tokens eligible for removal
  projectedPostTokens: number; // ctx size after the checkpoint would apply
  rTokens: number; // R: invalidated cache suffix
  sNetTokens: number; // S_net: tokens actually removed, net of overhead
  nonRecoverableKeptTokens: number;
  stubOverheadTokens: number;
  plateauTurns: number;
  errorSnippet: string | null;
}

// break-even future rounds for a given q (spec §2).
export function breakEvenTurns(q: number): number {
  return 11.5 / q - 12.5;
}

// q in time order — one growing conversation, so q rises monotonically but
// stays far under the GO line the whole way.
const Q_SERIES = [
  0.164, 0.181, 0.199, 0.208, 0.215, 0.229, 0.242, 0.25, 0.273, 0.312, 0.36,
  0.44,
];

const CREATED_AT = [
  "2026-07-12T03:28:11Z",
  "2026-07-12T03:31:39Z",
  "2026-07-12T03:34:02Z",
  "2026-07-12T03:36:47Z",
  "2026-07-12T03:39:20Z",
  "2026-07-12T03:41:05Z",
  "2026-07-12T03:43:58Z",
  "2026-07-12T03:45:40Z",
  "2026-07-12T03:48:51Z",
  "2026-07-12T03:51:33Z",
  "2026-07-12T03:54:17Z",
  "2026-07-12T03:57:39Z",
];

const MODELS = [
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-sonnet-5",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
  "claude-sonnet-5",
  "claude-opus-4-8[1m]",
  "claude-opus-4-8[1m]",
];

// Near-identical removal set (~9.3k) across the growing conversation — the
// candidate block barely moves while total ctx climbs, which is exactly why
// everything reads "saturated".
const GROSS_CANDIDATE = [
  9120, 9180, 9210, 9240, 9260, 9280, 9300, 9310, 9330, 9360, 9420, 9500,
];

export const SAMPLES: CheckpointSample[] = Q_SERIES.map((q, i) => {
  const rTokens = 36_000 + i * 320; // invalidated suffix grows with the ctx
  const sNet = Math.round(q * rTokens); // q = s_net / r, by construction
  const gross = GROSS_CANDIDATE[i];
  const stubOverhead = 210 + (i % 4) * 12;
  const nonRecoverableKept = gross - sNet > 0 ? gross - sNet : 0;
  return {
    id: 101 + i,
    createdAt: CREATED_AT[i],
    model: MODELS[i],
    q,
    outcome: "saturated" as Outcome,
    countFailure: 0,
    grossCandidateTokens: gross,
    projectedPostTokens: CEILING_TOKENS + 8_000 + i * 900 - sNet,
    rTokens,
    sNetTokens: sNet,
    nonRecoverableKeptTokens: nonRecoverableKept,
    stubOverheadTokens: stubOverhead,
    plateauTurns: 6 + (i % 3),
    errorSnippet: null,
  };
});

export interface Aggregates {
  count: number;
  qAvg: number;
  qMin: number;
  qMax: number;
  breakEvenMin: number;
  breakEvenMax: number;
  breakEvenAtAvg: number;
  countFailures: number;
  outcomeCounts: Record<Outcome, number>;
  verdict: "GO" | "NO-GO";
  gapToGo: number; // GO_THRESHOLD - qAvg, how far short the average sits
  requiredMultiple: number; // qAvg must ~x this to reach the GO line
  latest: CheckpointSample;
}

export function aggregate(samples: CheckpointSample[]): Aggregates {
  const qs = samples.map((s) => s.q);
  const qAvg = qs.reduce((a, b) => a + b, 0) / qs.length;
  const qMin = Math.min(...qs);
  const qMax = Math.max(...qs);
  const outcomeCounts: Record<Outcome, number> = {
    below_ceiling: 0,
    eligible: 0,
    saturated: 0,
  };
  for (const s of samples) outcomeCounts[s.outcome] += 1;
  return {
    count: samples.length,
    qAvg,
    qMin,
    qMax,
    breakEvenMin: breakEvenTurns(qMax), // higher q -> fewer turns
    breakEvenMax: breakEvenTurns(qMin),
    breakEvenAtAvg: breakEvenTurns(qAvg),
    countFailures: samples.filter((s) => s.countFailure === 1).length,
    outcomeCounts,
    verdict: qAvg >= GO_THRESHOLD ? "GO" : "NO-GO",
    gapToGo: GO_THRESHOLD - qAvg,
    requiredMultiple: GO_THRESHOLD / qAvg,
    latest: samples[samples.length - 1],
  };
}

// q-distribution buckets of width 0.1 across [0, 1] for the histogram.
export interface QBucket {
  label: string; // e.g. "0.2"
  lo: number;
  hi: number;
  count: number;
  containsThreshold: boolean;
}

export function qBuckets(samples: CheckpointSample[]): QBucket[] {
  const buckets: QBucket[] = [];
  for (let i = 0; i < 10; i++) {
    const lo = i / 10;
    const hi = lo + 0.1;
    buckets.push({
      label: lo.toFixed(1),
      lo,
      hi,
      count: samples.filter((s) => s.q >= lo && s.q < hi).length,
      containsThreshold: GO_THRESHOLD >= lo && GO_THRESHOLD < hi,
    });
  }
  return buckets;
}

// Points for the q-over-time trend, indexed by sample order.
export interface TrendPoint {
  n: number; // sample number, 1-based
  q: number;
  time: string; // HH:MM (frozen)
}

export function qTrend(samples: CheckpointSample[]): TrendPoint[] {
  return samples.map((s, i) => ({
    n: i + 1,
    q: s.q,
    time: s.createdAt.slice(11, 16),
  }));
}

export function fmtPct(q: number, digits = 1): string {
  return `${(q * 100).toFixed(digits)}%`;
}

export function fmtQ(q: number): string {
  return q.toFixed(3);
}

export function fmtTurns(n: number): string {
  return `${Math.round(n)}`;
}

export function fmtTokens(t: number): string {
  return t >= 1000 ? `${(t / 1000).toFixed(1)}k` : `${t}`;
}

export function fmtClock(iso: string): string {
  return iso.slice(11, 16); // HH:MM (UTC)
}
