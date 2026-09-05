#!/usr/bin/env node
// uishot — screenshot the REAL src/ app (fixture mode) so an agent can see the
// pixels it just changed. Port of arta's headless-snapshot core; see
// docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";

const BASE = "http://localhost:1420";
const SENTINEL = 'body[data-conclave-ready="1"]';

// --- arg parsing (no deps) ---
const args = process.argv.slice(2);
const valueOptions = new Set(["scenario", "out", "viewport", "theme", "timezone"]);
const options = new Map();
let view;
let full = false;
for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];
  if (arg === "--full") {
    full = true;
    continue;
  }
  if (arg.startsWith("--")) {
    const name = arg.slice(2);
    if (!valueOptions.has(name)) {
      console.error(`[uishot] unknown option ${arg}`);
      process.exit(2);
    }
    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
      console.error(`[uishot] ${arg} requires a value`);
      process.exit(2);
    }
    options.set(name, value);
    index += 1;
    continue;
  }
  if (view) {
    console.error(`[uishot] unexpected argument ${arg}`);
    process.exit(2);
  }
  view = arg;
}
if (!view) {
  console.error(
    "usage: pnpm uishot <viewId> [--scenario <name>] [--theme light|dark] [--timezone <IANA>] [--full] [--out <path>] [--viewport WxH]",
  );
  console.error(
    "  viewIds: overview workspaces archived workspace-settings home laneboard memory artifacts blackboard chat library builder builder-edit browser settings",
  );
  process.exit(2);
}
const validViews = new Set([
  "overview", "workspaces", "archived", "workspace-settings", "home", "laneboard",
  "memory", "artifacts", "blackboard", "chat", "library", "builder", "builder-edit",
  "browser", "settings",
]);
if (!validViews.has(view)) {
  console.error(`[uishot] unknown view ${view}`);
  process.exit(2);
}
const scenario = options.get("scenario") ?? "default";
const theme = options.get("theme");
if (theme && theme !== "light" && theme !== "dark") {
  console.error(`[uishot] --theme must be light or dark`);
  process.exit(2);
}
const timeZone = options.get("timezone") ?? "Asia/Bangkok";
const viewport = options.get("viewport") ?? "1440x900";
if (!/^\d+x\d+$/.test(viewport)) {
  console.error(`[uishot] --viewport must look like 1440x900`);
  process.exit(2);
}
const [vw, vh] = viewport.split("x").map(Number);
if (vw <= 0 || vh <= 0) {
  console.error(`[uishot] viewport dimensions must be positive`);
  process.exit(2);
}
const themeSuffix = theme ? `-${theme}` : "";
const out = options.get("out") ?? path.join(".shots", `${view}-${scenario}${themeSuffix}.png`);

const expectedState = view === "overview"
  ? scenario === "usage-loading" ? "loading" : scenario === "usage-error" ? "error" : "ready"
  : view === "workspaces" || view === "archived" || view === "workspace-settings"
    ? scenario === "workspace-loading" ? "loading" : scenario === "workspace-error" ? "error" : "ready"
    : null;

// Common install locations for a Chromium-family browser, per platform.
// Verbatim port of arta's vite/headless-snapshot.ts findChrome table.
function findChrome() {
  const env = process.env.PUPPETEER_EXECUTABLE_PATH || process.env.CHROME_PATH;
  if (env && fs.existsSync(env)) return env;
  const byPlatform = {
    darwin: [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
      "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    ],
    win32: [
      "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
      "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
      "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    ],
    linux: [
      "/usr/bin/google-chrome",
      "/usr/bin/google-chrome-stable",
      "/usr/bin/chromium",
      "/usr/bin/chromium-browser",
      "/snap/bin/chromium",
      "/usr/bin/microsoft-edge",
    ],
  };
  const list = byPlatform[process.platform] || byPlatform.linux;
  return list.find((p) => fs.existsSync(p)) || null;
}

// Ensure the Vite dev server on 1420 is up: reuse if already running, else
// start it detached (so it survives after uishot exits) and poll until ready.
// Vite's strictPort means a stale server from another lane's checkout may be
// what answers here — printing reuse-vs-started lets the caller tell.
async function ensureServer() {
  const alive = await fetch(BASE, { signal: AbortSignal.timeout(1000) })
    .then(() => true)
    .catch(() => false);
  if (alive) {
    console.log("[uishot] reusing dev server");
    return;
  }
  const child = spawn("pnpm", ["dev"], { detached: true, stdio: "ignore" });
  child.unref();
  console.log(`[uishot] started dev server (pnpm dev, pid ${child.pid})`);
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const up = await fetch(BASE, { signal: AbortSignal.timeout(1000) })
      .then(() => true)
      .catch(() => false);
    if (up) return;
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("dev server did not become ready within 30s");
}

// Expand inner scroll regions + re-root viewport-anchored bars to the full
// document, so a fullPage shot captures the WHOLE content. Verbatim port of
// arta's vite/headless-snapshot.ts UNCLAMP_IN_PAGE. No restore needed — the
// page is thrown away after the shot.
const UNCLAMP_IN_PAGE = `(function(){
  var root=document.documentElement, scrollers=[], fixedBars=[];
  root.querySelectorAll('*').forEach(function(el){
    var cs=window.getComputedStyle(el);
    if((cs.overflowY==='auto'||cs.overflowY==='scroll') && el.scrollHeight>el.clientHeight+4) scrollers.push(el);
    if(cs.position==='fixed') fixedBars.push(el);
  });
  var set=function(el,s){ for(var k in s) el.style[k]=s[k]; };
  var U={flex:'none',height:'auto',maxHeight:'none',minHeight:'0',bottom:'auto'};
  set(document.body,U);
  scrollers.forEach(function(sc){ var n=sc; while(n&&n!==root){ set(n,U); n=n.parentElement; } });
  set(root,Object.assign({position:'relative'},U));
  fixedBars.forEach(function(el){ set(el,{position:'absolute'}); });
})()`;

const puppeteer = (await import("puppeteer-core")).default;
const executablePath = findChrome();
if (!executablePath) {
  console.error("[uishot] no Chrome found — install Chrome or set CHROME_PATH");
  process.exit(1);
}
await ensureServer();

const browser = await puppeteer.launch({
  executablePath,
  headless: true,
  args: ["--no-sandbox", "--hide-scrollbars", "--force-color-profile=srgb", "--disable-gpu"],
});
let failed = false;
try {
  const page = await browser.newPage();
  await page.setViewport({ width: vw, height: vh, deviceScaleFactor: 2 });
  try {
    await page.emulateTimezone(timeZone);
  } catch (error) {
    console.error(`[uishot] invalid or unsupported timezone ${timeZone}: ${error.message}`);
    process.exitCode = 2;
    failed = true;
  }
  if (theme) {
    await page.evaluateOnNewDocument((nextTheme) => {
      localStorage.setItem("conclave.theme", nextTheme);
    }, theme);
  }
  // A component that CATCHES a fixture throw and only console.error's it used
  // to pass green — collect error-type messages and any `[fixture]` hit (not
  // warn or lower) so the run can fail AFTER the shot is written.
  const consoleFails = [];
  page.on("console", (m) => {
    const text = m.text();
    if (m.type() === "error") console.log(`[page] console.error: ${text}`);
    if (m.type() === "error" || text.includes("[fixture]")) {
      consoleFails.push(`${m.type()}: ${text}`);
    }
  });
  page.on("pageerror", (e) => {
    failed = true;
    console.log(`[page] pageerror: ${e.message}`);
  });
  const url = `${BASE}/?fixture=${encodeURIComponent(scenario)}#view=${encodeURIComponent(view)}`;
  await page.goto(url, { waitUntil: "load", timeout: 20000 });
  const escapedView = view.replaceAll('"', '\\"');
  const exactSentinel = `${SENTINEL}[data-conclave-view="${escapedView}"]${
    expectedState ? `[data-conclave-state="${expectedState}"]` : ""
  }`;
  const ready = await page
    .waitForSelector(exactSentinel, { timeout: 20000 })
    .then(() => true)
    .catch(() => false);
  if (!ready) {
    failed = true;
    console.log(`[uishot] readiness sentinel never appeared: ${exactSentinel}`);
  }
  if (view === "workspace-settings") {
    await page.waitForSelector('dialog.workspace-dialog[open]', { timeout: 5000 }).catch(() => {
      failed = true;
      console.log("[uishot] workspace settings dialog did not open");
    });
    if (
      scenario === "workspace-archive-error"
      || scenario === "workspace-archive-pending"
      || scenario === "workspace-busy"
    ) {
      await page.click('dialog.workspace-dialog [data-workspace-action="archive"]');
      const actionState = scenario === "workspace-archive-pending" ? "archive" : "error";
      await page.waitForSelector(`dialog.workspace-dialog[data-workspace-settings-state="${actionState}"]`, {
        timeout: 5000,
      });
    }
  }
  if (
    view === "archived"
    && (scenario === "workspace-restore-error" || scenario === "workspace-restore-pending")
  ) {
    await page.click('[data-workspace-action="restore"]');
    const restoreSelector = scenario === "workspace-restore-pending"
      ? '[data-workspace-action="restore"]:disabled'
      : '.workspace-manager-row-alert[role="alert"]';
    await page.waitForSelector(restoreSelector, { timeout: 5000 });
  }
  await new Promise((r) => setTimeout(r, 300)); // settle for image paints
  fs.mkdirSync(path.dirname(out), { recursive: true });
  if (full) {
    await page.evaluate(UNCLAMP_IN_PAGE);
    await new Promise((r) => setTimeout(r, 60)); // reflow
  }
  // A shot is still written on failure when possible — a broken screenshot is
  // evidence, the exit code carries the verdict.
  fs.writeFileSync(out, await page.screenshot({ type: "png", fullPage: full }));
  console.log(
    `[uishot] wrote ${out} (${vw}x${vh}@2x, scenario=${scenario}, theme=${theme ?? "system"}, timezone=${timeZone})`,
  );
  if (consoleFails.length > 0) {
    failed = true;
    for (const line of consoleFails) console.log(`[uishot] console-fail: ${line}`);
  }
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);
