#!/usr/bin/env node
// Deterministic regression gate for the design-host design-review grader. Runs grade()
// over the committed good/ and bad/ fixtures and checks each against thresholds.json:
//
//   good-fixture: every `require`d assertion must PASS.
//   bad-fixture:  every `mustFail` assertion must FAIL, every `mustPass` must PASS.
//
// Exit 0 iff both targets meet their threshold, non-zero otherwise. Local only — no CI
// wiring, no network, no bun.
//
//   node design-host/evals/gate.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { grade } from "../review/grade.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const thresholds = JSON.parse(fs.readFileSync(path.join(HERE, "thresholds.json"), "utf8"));

// Look up a check's pass state by id in a grade() result.
function checkPass(result, id) {
  const c = result.checks.find((x) => x.id === id);
  return c ? c.pass : undefined;
}

let ok = true;
const report = [];

for (const target of thresholds.targets) {
  const dir = path.resolve(HERE, target.dir);
  const result = grade(dir);
  const problems = [];

  for (const [id, want] of Object.entries(target.require || {})) {
    if (checkPass(result, id) !== want) problems.push(`${id} expected ${want}, got ${checkPass(result, id)}`);
  }
  for (const id of target.mustFail || []) {
    if (checkPass(result, id) !== false) problems.push(`${id} must FAIL but got ${checkPass(result, id)}`);
  }
  for (const id of target.mustPass || []) {
    if (checkPass(result, id) !== true) problems.push(`${id} must PASS but got ${checkPass(result, id)}`);
  }

  const targetOk = problems.length === 0;
  ok = ok && targetOk;
  report.push({ target, result, problems, targetOk });
}

// Summary.
for (const { target, result, problems, targetOk } of report) {
  console.log(`\n[${targetOk ? "OK" : "FAIL"}] ${target.id}  (${target.dir})  overall=${result.pass ? "pass" : "fail"} ${result.passed}/${result.nChecks}`);
  for (const c of result.checks) {
    const mark = c.skipped ? "skip" : c.pass ? "pass" : "fail";
    console.log(`    ${mark.padEnd(4)} ${c.id}`);
  }
  for (const p of problems) console.log(`    ✗ ${p}`);
}

console.log(`\n${ok ? "GATE PASS — both targets meet thresholds." : "GATE FAIL — see mismatches above."}`);
process.exit(ok ? 0 : 1);
