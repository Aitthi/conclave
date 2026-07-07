import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { createServer } from "vite";
import { transformSync } from "esbuild";

const pkgRoot = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const projectsSrc = fs.readFileSync(path.join(pkgRoot, "vite", "projects.ts"), "utf8");
const { idFor } = await import(
  `data:text/javascript,${encodeURIComponent(transformSync(projectsSrc, { loader: "ts" }).code)}`
);

function makeFixture() {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "dh-home-"));
  const design = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "dh-ws-")), "design");
  fs.mkdirSync(path.join(design, "screens"), { recursive: true });
  fs.mkdirSync(path.join(design, "public", "img"), { recursive: true });
  // Fake pre-installed ESM dep — auto-install is unit-tested; e2e only proves resolution.
  const dep = path.join(design, "node_modules", "demo-lib");
  fs.mkdirSync(dep, { recursive: true });
  fs.writeFileSync(path.join(dep, "package.json"), JSON.stringify({ name: "demo-lib", version: "1.0.0", type: "module", main: "index.js" }));
  fs.writeFileSync(path.join(dep, "index.js"), "export const label = 'from-demo-lib';");
  fs.writeFileSync(
    path.join(design, "screens", "demo.tsx"),
    "import { label } from 'demo-lib';\nexport default function Demo(){ return <div>{label}</div>; }\n",
  );
  fs.writeFileSync(path.join(design, "public", "img", "dot.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>");
  fs.writeFileSync(path.join(home, "registry.json"), JSON.stringify([{ id: "x", name: "fixture", dir: design }]));
  return { home, design, id: idFor(design) };
}

test("e2e: workspace dep resolves, missing dep errors, public assets guarded", async (t) => {
  const { home, design, id } = makeFixture();
  process.env.CONCLAVE_DESIGN_HOME = home;
  const server = await createServer({
    configFile: path.join(pkgRoot, "vite.config.ts"),
    root: pkgRoot,
    server: { port: 0 },
  });
  await server.listen();
  t.after(() => server.close());
  const base = `http://127.0.0.1:${server.config.server.port ?? server.httpServer.address().port}`;

  const manifest = await (await fetch(`${base}/@id/virtual:design-host-manifest/${id}`)).text();
  assert.match(manifest, /demo/);

  // The resolved bare import goes through Vite's own dep optimizer, so the
  // transformed screen code references a prebundled chunk (/node_modules/.vite/deps/…)
  // rather than the raw workspace path — follow that chunk and check ITS content
  // to prove the import resolved to the WORKSPACE's demo-lib, not a host copy.
  const screen = await server.transformRequest("/@fs" + path.join(design, "screens", "demo.tsx"));
  assert.ok(screen, "screen must transform");
  const depMatch = /\/node_modules\/\.vite\/deps\/demo-lib\.js\?v=[a-z0-9]+/.exec(screen.code);
  assert.ok(depMatch, "demo-lib import must resolve and get prebundled by Vite");
  const depCode = await (await fetch(`${base}${depMatch[0]}`)).text();
  assert.match(depCode, /from-demo-lib/, "resolved dep must be the workspace's own demo-lib");

  fs.writeFileSync(
    path.join(design, "screens", "broken.tsx"),
    // `x` must be referenced as a value — an unused single-file import binding
    // is ambiguous type-vs-value and esbuild's isolated TS transform elides it
    // entirely (never reaching resolveId), which would make this assertion moot.
    "import x from 'not-installed-lib';\nexport default () => x;\n",
  );
  await assert.rejects(
    () => server.transformRequest("/@fs" + path.join(design, "screens", "broken.tsx")),
    /not installed in this workspace/,
  );

  assert.equal((await fetch(`${base}/p/${id}/img/dot.svg`)).status, 200);
  assert.equal((await fetch(`${base}/p/${id}/img/dot.svg`)).headers.get("content-type"), "image/svg+xml");
  assert.equal((await fetch(`${base}/p/${id}/missing.png`)).status, 404);
  assert.equal((await fetch(`${base}/p/${id}/..%2Ftheme.css`)).status, 403);
  assert.equal((await fetch(`${base}/p/zzzz/anything.png`)).status, 404);
});
