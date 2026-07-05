#!/usr/bin/env node
// uishot-eval — smoke matrix for the uishot capture CLI. Runs every
// (view, scenario) cell, asserts the PNG landed and looks like a real
// screenshot, and reports the repeatable gate command for UI lanes:
// `node scripts/uishot-eval.mjs`. See
// docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const VIEWS = ["home", "laneboard"];
const SCENARIOS = ["default", "empty"];
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const MIN_SIZE = 10 * 1024;

let failures = 0;

for (const view of VIEWS) {
  for (const scenario of SCENARIOS) {
    const out = path.join(".shots", `${view}-${scenario}.png`);
    const result = spawnSync("pnpm", ["uishot", view, "--scenario", scenario, "--out", out], {
      stdio: "inherit",
    });

    const problems = [];
    if (result.status !== 0) problems.push(`exit code ${result.status}`);
    if (!fs.existsSync(out)) {
      problems.push("PNG missing");
    } else {
      const size = fs.statSync(out).size;
      if (size <= MIN_SIZE) problems.push(`PNG too small (${size} bytes)`);
      const head = Buffer.alloc(8);
      const fd = fs.openSync(out, "r");
      fs.readSync(fd, head, 0, 8, 0);
      fs.closeSync(fd);
      if (!head.equals(PNG_MAGIC)) problems.push("PNG magic bytes mismatch");
    }

    const cell = `${view}/${scenario}`;
    if (problems.length === 0) {
      console.log(`[uishot-eval] PASS ${cell}`);
    } else {
      console.log(`[uishot-eval] FAIL ${cell}: ${problems.join(", ")}`);
      failures++;
    }
  }
}

process.exit(failures > 0 ? 1 : 0);
