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
const view = args.find((a) => !a.startsWith("--"));
if (!view) {
  console.error(
    "usage: pnpm uishot <viewId> [--scenario default|empty] [--full] [--out <path>] [--viewport WxH]",
  );
  process.exit(2);
}
const opt = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : dflt;
};
const scenario = opt("scenario", "default");
const full = args.includes("--full");
const [vw, vh] = opt("viewport", "1440x900").split("x").map(Number);
const out = opt("out", path.join(".shots", `${view}-${scenario}.png`));

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
  const ready = await page
    .waitForSelector(SENTINEL, { timeout: 20000 })
    .then(() => true)
    .catch(() => false);
  if (!ready) {
    failed = true;
    console.log("[uishot] readiness sentinel never appeared (page crashed or fixture handler missing?)");
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
  console.log(`[uishot] wrote ${out} (${vw}x${vh}@2x, scenario=${scenario})`);
  if (consoleFails.length > 0) {
    failed = true;
    for (const line of consoleFails) console.log(`[uishot] console-fail: ${line}`);
  }
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);
