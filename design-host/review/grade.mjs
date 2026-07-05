#!/usr/bin/env node
// Deterministic design-QA grader for the Conclave design-host. Ported from Arta's
// evals/grade.mjs (MIT — Arta project), adapted to the FLAT `design/` root the
// design-host renders: instead of Arta's `proto/` subdir, sources live straight under
// the given <designDir>:
//
//   <designDir>/theme.css        — Tailwind v4 sheet with an @theme { --color-*: … } block
//   <designDir>/screens/*.tsx    — one default-exported React screen per file
//   <designDir>/components/*.tsx — optional shared components
//   <designDir>/config.json      — OPTIONAL { "start": "<screenId>" }
//   <designDir>/lib/             — optional shared code
//
// It scores SIX deterministic assertions straight off the on-disk sources — no LLM, no
// bundler, no headless browser:
//
//   A1a tokens-defined — theme.css declares a real @theme token set
//   A1b tokens-used    — screens don't hardcode raw hex past a small allowance
//   A2  shared-layout  — ≥1 components/ file is imported by ≥2 screens (vacuous-pass when
//                        there are 0 components or <2 screens: a fresh scaffold still PASSES)
//   A3  interactivity  — every screen reachable from config.start via <Link to>/navigate();
//                        ONLY runs when config.json exists — otherwise SKIPPED (not failed)
//   A4  renders-clean  — every screen/component parses as valid TSX (esbuild transformSync)
//                        and every screen has a default export
//   A5  design-review  — Arta's JSX-aware slop detector finds ZERO serious findings
//
// Runnable as:  node design-host/review/grade.mjs <designDir> [--json]
// and exposes a grade(designDir) function returning { score, passed, nChecks, checks,
// findings, metrics }.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { detectSlop, detectSlopJsx } from "./slop-detect.mjs";

// ── SERIOUS antipattern set (pinned R4; inlined — do NOT read Arta's briefs.json) ──────
// A finding gates A5 iff its `antipattern` is in this set. Everything else enriches the
// report without failing the design.
const SERIOUS = new Set([
  "gradient-text",
  "side-tab",
  "gpt-thin-border-wide-shadow",
  "repeating-stripes-gradient",
  "hero-eyebrow-chip",
  "cream-palette",
  "ai-color-palette",
  "nested-cards",
  "extreme-negative-tracking",
]);

const readRaw = (f) => {
  try {
    return fs.readFileSync(f, "utf8");
  } catch {
    return null;
  }
};

// ── tokensFromCss — minimal reimplementation of Arta's src/lib/prototype.ts parser ─────
// Parses @theme / :root custom properties out of theme.css text (no Tailwind compile),
// classifying by prefix (--color-*, --font-*, --radius-*, --text-*, …) then by value
// shape. Returns { colors, fonts, radii, shadows, spacing, typography } — arrays of
// { name, value }. We only need colors/fonts/typography counts for A1a, but the full
// classification keeps parity with Arta so the same theme.css grades identically.
const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const isColorVal = (v) =>
  /^#[0-9a-f]{3,8}$/i.test(v) ||
  /^(rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color|color-mix)\s*\(/i.test(v) ||
  /^(transparent|currentcolor|black|white|red|green|blue|orange|purple|pink|gray|grey|yellow|teal|cyan|magenta|indigo|violet|slate|navy|gold|brown|beige|cream|coral|salmon|lime|olive|maroon|aqua|silver|crimson|tomato|turquoise|azure|ivory|khaki|plum|tan|wheat)$/i.test(
    v,
  );
const isLenVal = (v) => /^-?[\d.]+(px|rem|em|%|vh|vw|vmin|vmax|pt|ch|ex|q|cm|mm|in|pc)$/i.test(v);
const looksShadow = (n, v) =>
  /shadow|elevation/i.test(n) || /\binset\b/i.test(v) || /(?:[\d.][\w.%-]*\s+){2,}(?:rgb|hsl|#|oklch|color)/i.test(v);
const looksFontStack = (n, v) =>
  /font|family|typeface/i.test(n) ||
  v.includes(",") ||
  /\b(serif|sans-serif|monospace|system-ui|ui-sans-serif|ui-serif|ui-monospace|cursive)\b/i.test(v);
const looksRadius = (n) => /radius|rounded|corner|radii/i.test(n) || /(^|[-_])r($|[-_\d])/i.test(n);

function tokensFromCss(css) {
  const out = {};
  if (!css || !css.trim()) return out;
  css = stripComments(css);
  // Only the LAST statement before a block's `{` is its real selector — theme.css stacks
  // several `;`-terminated at-rules (@import; @source; @custom-variant …;) before @theme.
  const lastStatement = (sel) => {
    const parts = sel.split(";");
    return parts[parts.length - 1].trim();
  };
  const isRootSel = (sel) =>
    lastStatement(sel)
      .split(",")
      .some((s) => {
        const t = s.trim();
        if (/\.dark\b/.test(t)) return false; // dark override, not the light palette
        return (
          t === ":root" ||
          t === "html" ||
          t === "body" ||
          t === ":host" ||
          /(^|[\s>~+]):root\b/.test(t) ||
          /^@theme\b/.test(t)
        );
      });
  const body = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => isRootSel(m[1]))
    .map((m) => m[2])
    .join(";");
  const seen = new Set();
  for (const m of body.matchAll(/--([\w-]+)\s*:\s*([^;]+)/g)) {
    const name = m[1].trim();
    const value = m[2].trim();
    if (!value || seen.has(name)) continue;
    seen.add(name);
    if (name.startsWith("color-")) (out.colors ||= []).push({ name: name.slice(6), value });
    else if (name.startsWith("font-")) (out.fonts ||= []).push({ name: name.slice(5), value });
    else if (name.startsWith("radius-")) (out.radii ||= []).push({ name: name.slice(7), value });
    else if (name.startsWith("shadow-")) (out.shadows ||= []).push({ name: name.slice(7), value });
    else if (name.startsWith("space-") || name.startsWith("spacing-"))
      (out.spacing ||= []).push({ name: name.replace(/^spac(?:e|ing)-/, ""), value });
    else if (name.startsWith("text-")) (out.typography ||= []).push({ name: name.slice(5), size: value });
    else if (looksShadow(name, value)) (out.shadows ||= []).push({ name, value });
    else if (isColorVal(value)) (out.colors ||= []).push({ name, value });
    else if (looksFontStack(name, value)) (out.fonts ||= []).push({ name, value });
    else if (looksRadius(name) && isLenVal(value)) (out.radii ||= []).push({ name, value });
    else if (isLenVal(value) && /space|spacing|gap|gutter|inset|pad|margin|size/i.test(name))
      (out.spacing ||= []).push({ name, value });
  }
  return out;
}

// ── manifest helpers — flat-root reimplementation of Arta's vite/proto-manifest.ts ─────
const sane = (s) => /^[a-z0-9_-]+$/i.test(s);
function listScreens(designDir) {
  const dir = path.join(designDir, "screens");
  let files = [];
  try {
    files = fs.readdirSync(dir);
  } catch {
    return [];
  }
  return files
    .filter((f) => f.endsWith(".tsx") && sane(f.slice(0, -4)))
    .sort()
    .map((f) => ({ id: f.slice(0, -4), file: path.join(dir, f) }));
}
function listComponents(designDir) {
  const dir = path.join(designDir, "components");
  try {
    return fs
      .readdirSync(dir)
      .filter((f) => f.endsWith(".tsx") && sane(f.slice(0, -4)))
      .sort()
      .map((f) => ({ name: f.slice(0, -4), file: path.join(dir, f) }));
  } catch {
    return [];
  }
}
// Returns { config, exists } — `exists` gates A3 (config.json ABSENT → A3 skipped).
function readConfig(designDir) {
  const file = path.join(designDir, "config.json");
  try {
    const cfg = JSON.parse(fs.readFileSync(file, "utf8"));
    return { config: cfg && typeof cfg === "object" ? cfg : {}, exists: true };
  } catch {
    return { config: {}, exists: fs.existsSync(file) };
  }
}

// ── local-import graph (regex, no bundler) ─────────────────────────────────────────────
const IMPORT_RE = /import\s+(?:[\w$*{},\s]+from\s+)?["']([^"']+)["']/g;
function importSpecifiers(src) {
  const out = [];
  let m;
  IMPORT_RE.lastIndex = 0;
  while ((m = IMPORT_RE.exec(src))) out.push(m[1]);
  return out;
}
const RESOLVE_EXTS = [".tsx", ".ts", ".jsx", ".js"];
function resolveLocalImport(fromFile, spec) {
  if (!spec.startsWith(".")) return null; // bare/package import — not part of the local graph
  const base = path.resolve(path.dirname(fromFile), spec);
  const candidates = [
    base,
    ...RESOLVE_EXTS.map((e) => base + e),
    ...RESOLVE_EXTS.map((e) => path.join(base, "index" + e)),
  ];
  for (const c of candidates) {
    try {
      if (fs.statSync(c).isFile()) return c;
    } catch {
      /* not this candidate */
    }
  }
  return null;
}
// Every local file transitively reachable from startFile via relative imports, bounded to
// `dir` so it can never wander outside the design. Cycle-safe.
function localClosure(startFile, dir, srcCache) {
  const seen = new Set();
  const stack = [startFile];
  while (stack.length) {
    const f = stack.pop();
    if (seen.has(f) || !f.startsWith(dir)) continue;
    seen.add(f);
    let src = srcCache.get(f);
    if (src === undefined) {
      src = readRaw(f);
      srcCache.set(f, src);
    }
    if (src == null) continue;
    for (const spec of importSpecifiers(src)) {
      const r = resolveLocalImport(f, spec);
      if (r && !seen.has(r)) stack.push(r);
    }
  }
  return seen;
}

// ── A3: literal nav targets — <Link to="…"> / <NavLink to="…"> / navigate("…") ─────────
const LINK_TO_RE = /(?<![\w-])to\s*=\s*(?:\{\s*)?["'`]([^"'`{}]+)["'`]/g;
const NAVIGATE_RE = /\bnavigate\s*\(\s*["'`]([^"'`]+)["'`]/g;
function navTargets(src) {
  const out = [];
  let m;
  LINK_TO_RE.lastIndex = 0;
  while ((m = LINK_TO_RE.exec(src))) out.push(m[1]);
  NAVIGATE_RE.lastIndex = 0;
  while ((m = NAVIGATE_RE.exec(src))) out.push(m[1]);
  return out;
}
// "/pricing#top?x=1" → "pricing"; external URL / same-page anchor → "" (dropped by caller).
function normTarget(t) {
  if (/^https?:\/\//i.test(t)) return "";
  const clean = t.split("#")[0].split("?")[0];
  return clean.replace(/^\/+/, "").split("/")[0];
}

/**
 * Grade a flat design/ root.
 * @param {string} designDir
 * @returns {{ designDir:string, score:number, passed:number, nChecks:number,
 *             pass:boolean, checks:Array<{id:string,pass:boolean,skipped:boolean,detail:string}>,
 *             findings:Array<object>, metrics:object }}
 */
export function grade(designDir) {
  const absDir = path.resolve(designDir);
  if (!fs.existsSync(path.join(absDir, "config.json")) && !fs.existsSync(path.join(absDir, "screens"))) {
    // No recognizable design tree — every assertion fails hard.
    const fatalChecks = [
      "A1a_tokens_defined",
      "A1b_tokens_used",
      "A2_shared",
      "A3_interactivity",
      "A4_render",
      "A5_design",
    ].map((id) => ({ id, pass: false, skipped: false, detail: "no design tree (config.json / screens/ missing)" }));
    return {
      designDir: absDir,
      score: 0,
      passed: 0,
      nChecks: fatalChecks.length,
      pass: false,
      checks: fatalChecks,
      findings: [],
      metrics: { fatal: "no design tree (config.json / screens/ missing)" },
    };
  }

  const { config, exists: hasConfig } = readConfig(absDir);
  const screenList = listScreens(absDir); // [{id, file}]
  const componentList = listComponents(absDir); // [{name, file}]
  const ids = new Set(screenList.map((s) => s.id));
  const srcCache = new Map(); // abs file -> source | null, shared across every closure walk
  const screenSrc = {};
  for (const s of screenList) screenSrc[s.id] = readRaw(s.file);

  const metrics = {};

  // ── A1a tokens-defined — theme.css's @theme block ────────────────────────────────
  const themeFile = path.join(absDir, "theme.css");
  const themeCssRaw = readRaw(themeFile) || "";
  const tokens = tokensFromCss(themeCssRaw);
  const nColors = (tokens.colors || []).length;
  const nType = (tokens.typography || []).length;
  const nFonts = (tokens.fonts || []).length;
  metrics.tokens = { nColors, nType, nFonts };
  const A1a = nColors >= 4 && (nType >= 2 || nFonts >= 2);
  const A1aDetail = `${nColors} colours, ${nType} type + ${nFonts} font tokens (need ≥4 colours and ≥2 type or ≥2 font)`;

  // ── A1b tokens-used — raw hex across each screen's render surface (its source PLUS the
  // local components/lib it imports) shouldn't dominate; a few one-off accents are fine.
  let screenRawHex = 0;
  for (const s of screenList) {
    if (screenSrc[s.id] == null) continue;
    const closure = localClosure(s.file, absDir, srcCache);
    let text = "";
    for (const f of closure) text += (srcCache.get(f) || "") + "\n";
    screenRawHex += (text.match(/#[0-9a-fA-F]{3,8}\b/g) || []).length;
  }
  const hexAllowance = 4 * Math.max(1, screenList.length);
  metrics.tokensUsed = { screenRawHex, hexAllowance };
  const A1b = screenRawHex <= hexAllowance;
  const A1bDetail = `${screenRawHex} raw hex literal(s) across screens (allowance ${hexAllowance})`;

  // ── A2 shared-layout — a components/ file DIRECTLY imported by ≥2 screens. Vacuous-pass
  // when there are 0 components or <2 screens (a fresh scaffold with one screen and no
  // components MUST still PASS overall) — R4. ──────────────────────────────────────────
  const componentFiles = new Set(componentList.map((c) => c.file));
  const usedBy = {}; // component file -> Set<screenId>
  for (const s of screenList) {
    if (screenSrc[s.id] == null) continue;
    for (const spec of importSpecifiers(screenSrc[s.id])) {
      const r = resolveLocalImport(s.file, spec);
      if (r && componentFiles.has(r)) (usedBy[r] ||= new Set()).add(s.id);
    }
  }
  const sharedComponents = Object.entries(usedBy)
    .filter(([, screens]) => screens.size >= 2)
    .map(([f, screens]) => ({ file: path.relative(absDir, f), screens: [...screens] }));
  const a2Vacuous = componentList.length < 1 || screenList.length < 2;
  metrics.shared = { componentCount: componentList.length, sharedComponents, vacuous: a2Vacuous };
  const A2 = a2Vacuous || sharedComponents.length >= 1;
  const A2Detail = a2Vacuous
    ? `vacuous pass (${componentList.length} components, ${screenList.length} screens — sharing not possible)`
    : sharedComponents.length >= 1
      ? `${sharedComponents.length} component(s) shared by ≥2 screens`
      : "no components/ file imported by ≥2 screens";

  // ── A3 interactivity — every screen reachable from config.start over <Link to>/
  // navigate() targets, walking each screen's local-import closure so nav in a SHARED
  // component counts too. ONLY runs when config.json exists; otherwise SKIPPED. ─────────
  let A3 = true;
  const A3Skipped = !hasConfig;
  let A3Detail = "skipped — no config.json (A3 only runs with a start screen wired)";
  if (!A3Skipped) {
    const adj = {};
    const rawTargets = {};
    for (const s of screenList) {
      if (screenSrc[s.id] == null) {
        adj[s.id] = [];
        rawTargets[s.id] = [];
        continue;
      }
      const closure = localClosure(s.file, absDir, srcCache);
      const targets = [];
      for (const f of closure) targets.push(...navTargets(srcCache.get(f) || ""));
      const norm = [...new Set(targets.map(normTarget).filter(Boolean))];
      rawTargets[s.id] = norm;
      adj[s.id] = norm.filter((t) => ids.has(t));
    }
    const allTargets = Object.values(rawTargets).flat();
    const badNav = [...new Set(allTargets)].filter((t) => t && !ids.has(t));
    const start = config.start && ids.has(config.start) ? config.start : screenList[0]?.id;
    const seen = new Set(start ? [start] : []);
    const stack = start ? [start] : [];
    while (stack.length) {
      const n = stack.pop();
      for (const t of adj[n] || []) if (!seen.has(t)) { seen.add(t); stack.push(t); }
    }
    const unreachable = screenList.map((s) => s.id).filter((id) => !seen.has(id));
    metrics.interactivity = { start, navCount: allTargets.length, badNav, unreachable };
    A3 = allTargets.length >= 1 && badNav.length === 0 && unreachable.length === 0;
    A3Detail = `start=${start}, ${allTargets.length} nav target(s), ${badNav.length} broken, ${unreachable.length} unreachable`;
  } else {
    metrics.interactivity = { skipped: true };
  }

  // ── A4 renders-clean — every screen/component parses as valid TSX (esbuild transformSync)
  // and every screen has a default export. (Arta's optional headless-Chrome live half is
  // dropped entirely.) ─────────────────────────────────────────────────────────────────
  const renderIssues = [];
  const checkParses = (id, file, src, kind) => {
    if (src == null) {
      renderIssues.push({ id, issue: `${kind} unreadable`, file: path.relative(absDir, file) });
      return;
    }
    try {
      transformSync(src, { loader: "tsx", jsx: "automatic" });
    } catch (e) {
      renderIssues.push({
        id,
        issue: `${kind} failed to parse`,
        file: path.relative(absDir, file),
        message: String(e?.message || e).split("\n")[0],
      });
      return;
    }
    if (kind === "screen" && !/export\s+default\b/.test(src))
      renderIssues.push({
        id,
        issue: "screen has no default export (can never mount)",
        file: path.relative(absDir, file),
      });
  };
  for (const s of screenList) checkParses(s.id, s.file, screenSrc[s.id], "screen");
  for (const c of componentList) checkParses(c.name, c.file, readRaw(c.file), "component");
  metrics.render = { screens: screenList.length, components: componentList.length, issues: renderIssues };
  const A4 = renderIssues.length === 0 && screenList.length >= 1;
  const A4Detail =
    renderIssues.length === 0
      ? `${screenList.length} screen(s) + ${componentList.length} component(s) parse clean with default exports`
      : `${renderIssues.length} render issue(s): ${renderIssues.map((r) => `${r.file} ${r.issue}`).join("; ")}`;

  // ── A5 design-review — Arta's JSX-aware slop detector over every screen + component
  // source, plus theme.css (plain-CSS detectSlop). Only SERIOUS findings gate. ──────────
  let findings = [];
  for (const s of screenList)
    if (screenSrc[s.id] != null) findings.push(...detectSlopJsx(screenSrc[s.id], { file: path.relative(absDir, s.file) }));
  for (const c of componentList) {
    const src = readRaw(c.file);
    if (src != null) findings.push(...detectSlopJsx(src, { file: path.relative(absDir, c.file) }));
  }
  if (themeCssRaw) findings.push(...detectSlop(themeCssRaw, { file: "theme.css" }));
  const serious = findings.filter((f) => SERIOUS.has(f.antipattern));
  const byId = {};
  for (const f of findings) byId[f.antipattern] = (byId[f.antipattern] || 0) + 1;
  metrics.designReview = { total: findings.length, byId, seriousCount: serious.length };
  const A5 = serious.length === 0;
  const A5Detail =
    serious.length === 0
      ? `${findings.length} finding(s), 0 serious`
      : `${serious.length} serious finding(s): ${[...new Set(serious.map((f) => f.antipattern))].join(", ")}`;

  const checks = [
    { id: "A1a_tokens_defined", pass: A1a, skipped: false, detail: A1aDetail },
    { id: "A1b_tokens_used", pass: A1b, skipped: false, detail: A1bDetail },
    { id: "A2_shared", pass: A2, skipped: false, detail: A2Detail },
    { id: "A3_interactivity", pass: A3, skipped: A3Skipped, detail: A3Detail },
    { id: "A4_render", pass: A4, skipped: false, detail: A4Detail },
    { id: "A5_design", pass: A5, skipped: false, detail: A5Detail },
  ];

  const counted = checks.filter((c) => !c.skipped);
  const passed = counted.filter((c) => c.pass).length;
  const nChecks = counted.length;
  // Overall pass = no serious findings AND every non-skipped assertion passes.
  const pass = serious.length === 0 && counted.every((c) => c.pass);

  return {
    designDir: absDir,
    score: nChecks ? passed / nChecks : 0,
    passed,
    nChecks,
    pass,
    checks,
    // Serious A5 findings only, in the pinned finding shape.
    findings: serious.map((f) => ({
      antipattern: f.antipattern,
      severity: f.severity,
      file: f.file,
      line: f.line,
      snippet: f.snippet,
      message: f.message,
    })),
    metrics,
  };
}

// ── CLI ────────────────────────────────────────────────────────────────────────────────
const isMain = (() => {
  try {
    return process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
})();
if (isMain) {
  const args = process.argv.slice(2);
  const designDir = args.find((a) => !a.startsWith("--"));
  const json = args.includes("--json");
  if (!designDir) {
    console.error("usage: node design-host/review/grade.mjs <designDir> [--json]");
    process.exit(2);
  }
  const result = grade(path.resolve(designDir));
  if (json) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    console.log(`design: ${result.designDir}`);
    console.log(`score:  ${result.passed}/${result.nChecks}  (${result.pass ? "PASS" : "FAIL"})`);
    for (const c of result.checks) {
      const mark = c.skipped ? "skip" : c.pass ? "PASS" : "FAIL";
      console.log(`  [${mark}] ${c.id} — ${c.detail}`);
    }
  }
  process.exit(result.pass ? 0 : 1);
}
