import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/workspace-install.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { parseWorkableVersion, needsInstall, writeMarker } = await import(moduleUrl);

test("parseWorkableVersion tolerates rc-file noise, rejects junk", () => {
  assert.ok(parseWorkableVersion("11.5.2\n"));
  assert.ok(parseWorkableVersion("nvm is loading...\n11.5.2\n"));
  assert.ok(!parseWorkableVersion(""));
  assert.ok(!parseWorkableVersion("TypeError: Invalid host defined options"));
});

test("needsInstall: no package.json → false; fresh → true; marker current → false; pkg edited → true", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "dh-"));
  assert.equal(needsInstall(dir), false);
  fs.writeFileSync(path.join(dir, "package.json"), '{"dependencies":{}}');
  assert.equal(needsInstall(dir), true);
  fs.mkdirSync(path.join(dir, "node_modules"), { recursive: true });
  writeMarker(dir);
  assert.equal(needsInstall(dir), false);
  const future = new Date(Date.now() + 5000);
  fs.utimesSync(path.join(dir, "package.json"), future, future);
  assert.equal(needsInstall(dir), true);
});
