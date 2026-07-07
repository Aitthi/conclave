import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/curated.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { CURATED, packageName, isCurated } = await import(moduleUrl);

test("curated list is the R6 set, unchanged", () => {
  assert.deepEqual(CURATED, [
    "react", "react-dom", "react-router-dom", "motion",
    "lucide-react", "recharts", "clsx", "tailwind-merge",
  ]);
});

test("packageName handles plain, subpath, and scoped specifiers", () => {
  assert.equal(packageName("react"), "react");
  assert.equal(packageName("react-dom/client"), "react-dom");
  assert.equal(packageName("@tanstack/react-table"), "@tanstack/react-table");
  assert.equal(packageName("@tanstack/react-table/build/lib"), "@tanstack/react-table");
});

test("isCurated matches by package name, not prefix", () => {
  assert.ok(isCurated("react-dom/client"));
  assert.ok(!isCurated("react-table"));
  assert.ok(!isCurated("@tanstack/react-table"));
});
