import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/public-assets.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { resolvePublicPath, contentTypeFor } = await import(moduleUrl);

test("resolvePublicPath stays inside design/public", () => {
  assert.equal(resolvePublicPath("/w/design", "logo.png"), "/w/design/public/logo.png");
  assert.equal(resolvePublicPath("/w/design", "img/a.svg"), "/w/design/public/img/a.svg");
  assert.equal(resolvePublicPath("/w/design", "../theme.css"), null);
  assert.equal(resolvePublicPath("/w/design", "..%2F..%2Fetc/passwd"), null);
  assert.equal(resolvePublicPath("/w/design", "a/../../secret"), null);
  assert.equal(resolvePublicPath("/w/design", ""), null);
});

test("contentTypeFor known and unknown extensions", () => {
  assert.equal(contentTypeFor("/x/logo.png"), "image/png");
  assert.equal(contentTypeFor("/x/font.woff2"), "font/woff2");
  assert.equal(contentTypeFor("/x/blob.xyz"), "application/octet-stream");
});
