#!/usr/bin/env node
// design-host design-review entry — the process the engine spawns. PINNED interface (an
// external Rust caller depends on it exactly):
//
//   node design-host/review/review.mjs <designDir> [--json]
//
// WITHOUT --json: prints a readable per-assertion table + any serious findings to stdout.
//   Exit 0 if the design passes; non-zero on any serious finding or failed (non-skipped)
//   assertion.
//
// WITH --json: prints EXACTLY ONE JSON object to stdout and nothing else, then exits.
//   Shape (PINNED):
//     { "pass": <bool>,
//       "findings": [ { antipattern, severity, file, line, snippet, message }, … ],
//       "assertions": [ { "id", "label", "pass": <bool>, "detail" }, … ] }
//   findings  = the A5 serious slop findings.
//   assertions = one row per assertion. A SKIPPED A3 (no config.json) is INCLUDED with
//     pass:true and a detail noting it was skipped — the design-host convention is to keep
//     all six rows stable for the caller so a missing config.json never changes the row
//     count; a skipped row never counts against `pass`.
//   pass = (no serious findings) AND (every non-skipped assertion passes).
//   In --json mode nothing but that JSON goes to stdout — all diagnostics go to stderr,
//   because the engine does JSON.parse(stdout).
import path from "node:path";
import { fileURLToPath } from "node:url";
import { grade } from "./grade.mjs";

// Human-readable labels for each assertion id (stable, for the JSON `label` and the table).
const LABELS = {
  A1a_tokens_defined: "Tokens defined",
  A1b_tokens_used: "Tokens used (raw-hex budget)",
  A2_shared: "Shared components",
  A3_interactivity: "Interactivity / navigation",
  A4_render: "Renders clean (valid TSX)",
  A5_design: "Design review (slop)",
};

function main() {
  const args = process.argv.slice(2);
  const json = args.includes("--json");
  const designDir = args.find((a) => !a.startsWith("--"));
  if (!designDir) {
    // usage errors go to stderr in both modes so --json stdout stays parseable.
    process.stderr.write("usage: node design-host/review/review.mjs <designDir> [--json]\n");
    process.exit(2);
  }

  let result;
  try {
    result = grade(path.resolve(designDir));
  } catch (e) {
    process.stderr.write(`design review failed: ${String(e?.stack || e)}\n`);
    process.exit(2);
  }

  const assertions = result.checks.map((c) => ({
    id: c.id,
    label: LABELS[c.id] || c.id,
    pass: c.pass,
    detail: c.skipped ? `(skipped) ${c.detail}` : c.detail,
  }));

  const pass = result.pass;

  if (json) {
    // EXACTLY one JSON object on stdout, nothing else.
    process.stdout.write(
      JSON.stringify({
        pass,
        findings: result.findings,
        assertions,
      }) + "\n",
    );
    process.exit(pass ? 0 : 1);
  }

  // Readable table mode.
  const lines = [];
  lines.push(`Design review: ${result.designDir}`);
  lines.push(`Result: ${pass ? "PASS" : "FAIL"}  (${result.passed}/${result.nChecks} assertions)`);
  lines.push("");
  for (const c of result.checks) {
    const mark = c.skipped ? "SKIP" : c.pass ? "PASS" : "FAIL";
    lines.push(`  [${mark}]  ${c.id.padEnd(20)} ${LABELS[c.id] || ""}`);
    lines.push(`          ${c.detail}`);
  }
  if (result.findings.length) {
    lines.push("");
    lines.push(`Serious findings (${result.findings.length}):`);
    for (const f of result.findings) {
      lines.push(`  • ${f.antipattern} [${f.severity}] ${f.file}:${f.line}`);
      lines.push(`      ${f.message}`);
      if (f.snippet) lines.push(`      ${String(f.snippet).replace(/\s+/g, " ").trim().slice(0, 100)}`);
    }
  }
  lines.push("");
  process.stdout.write(lines.join("\n") + "\n");
  process.exit(pass ? 0 : 1);
}

// Run only when invoked directly.
const isMain = (() => {
  try {
    return process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
})();
if (isMain) main();
