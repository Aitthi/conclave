import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../src/screenSelection.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { parseHashScreen, pickInitialScreen } = await import(moduleUrl);

test("hash wins over stored", () => {
  assert.equal(pickInitialScreen("b", "c", ["a", "b", "c"]), "b");
});

test("stored wins over default when hash invalid", () => {
  assert.equal(pickInitialScreen("gone", "c", ["a", "b", "c"]), "c");
});

test("falls back to welcome, then first, then null", () => {
  assert.equal(pickInitialScreen(null, null, ["x", "welcome"]), "welcome");
  assert.equal(pickInitialScreen(null, null, ["x", "y"]), "x");
  assert.equal(pickInitialScreen(null, null, []), null);
});

test("parseHashScreen", () => {
  assert.equal(parseHashScreen("#/lane-board"), "lane-board");
  assert.equal(parseHashScreen(""), null);
  assert.equal(parseHashScreen("#other"), null);
});
