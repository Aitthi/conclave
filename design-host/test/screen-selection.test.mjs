import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../src/screenSelection.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { filterScreens, parseHashScreen, pickInitialScreen } = await import(moduleUrl);

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

test("filterScreens: empty query returns every id, untouched order", () => {
  const ids = ["welcome", "lane-board", "Settings"];
  assert.deepEqual(filterScreens(ids, ""), ids);
});

test("filterScreens: whitespace-only query is treated as empty", () => {
  const ids = ["welcome", "lane-board"];
  assert.deepEqual(filterScreens(ids, "   "), ids);
});

test("filterScreens: case-insensitive substring match on both sides", () => {
  const ids = ["welcome", "lane-board", "Settings"];
  assert.deepEqual(filterScreens(ids, "BOARD"), ["lane-board"]);
  assert.deepEqual(filterScreens(ids, "set"), ["Settings"]);
  assert.deepEqual(filterScreens(ids, "e"), ["welcome", "lane-board", "Settings"]);
});

test("filterScreens: surrounding whitespace is trimmed before matching", () => {
  assert.deepEqual(filterScreens(["welcome", "lane-board"], "  lane "), ["lane-board"]);
});

test("filterScreens: no match yields an empty list", () => {
  assert.deepEqual(filterScreens(["welcome", "lane-board"], "zzz"), []);
  assert.deepEqual(filterScreens([], "anything"), []);
});
