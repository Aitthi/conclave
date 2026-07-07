import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/workspace-deps.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { isBareImport, projectDirFor, missingDepMessage } = await import(moduleUrl);

test("isBareImport", () => {
  assert.ok(isBareImport("@tanstack/react-table"));
  assert.ok(isBareImport("date-fns/format"));
  assert.ok(!isBareImport("./sidebar"));
  assert.ok(!isBareImport("../lib/cn"));
  assert.ok(!isBareImport("/@fs/Users/x/design/screens/a.tsx"));
  assert.ok(!isBareImport("virtual:design-host-manifest/abc"));
  assert.ok(!isBareImport("\0virtual:design-host-manifest/abc"));
});

test("projectDirFor maps /@fs/ and plain absolute importers to their registered dir", () => {
  const projects = [{ dir: "/work/ket-doc/design" }, { dir: "/work/other/design" }];
  assert.equal(projectDirFor("/@fs//work/ket-doc/design/screens/app.tsx", projects), "/work/ket-doc/design");
  assert.equal(projectDirFor("/work/other/design/components/x.tsx?t=123", projects), "/work/other/design");
  assert.equal(projectDirFor("/somewhere/else/file.tsx", projects), null);
  assert.equal(projectDirFor(undefined, projects), null);
});

test("missingDepMessage names the package and the exact file to edit", () => {
  const msg = missingDepMessage("@tanstack/react-table", "/work/ket-doc/design");
  assert.match(msg, /@tanstack\/react-table/);
  assert.match(msg, /\/work\/ket-doc\/design\/package\.json/);
  assert.match(msg, /installed automatically/);
});
