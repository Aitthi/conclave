#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PORT = Number(process.env.MEMORY_GRAPH_STRESS_PORT ?? 1421);
const BASE = `http://127.0.0.1:${PORT}`;
const NODE_COUNT = 277;
const EDGE_COUNT = 584;
const VIEWPORT = { width: 1440, height: 900, deviceScaleFactor: 1 };
const SHOTS = path.join(ROOT, ".shots");

const CURRENT_SOURCES = [
  "fx-ag-detoro",
  "fx-ag-mellow",
  "fx-ag-tiesto",
  "fx-ag-dew",
];
const FORMER_SOURCES = [
  "deadbeef-0000-4000-8000-000000000001",
  "deadbeef-1111-4000-8000-000000000002",
  ...Array.from(
    { length: 18 },
    (_, i) => `${(0xf0000000 + i).toString(16)}-0000-4000-8000-${String(i + 3).padStart(12, "0")}`,
  ),
];

function stressGraph() {
  const nodes = Array.from({ length: NODE_COUNT }, (_, i) => {
    const bucket = i % 25;
    const current = CURRENT_SOURCES[i % CURRENT_SOURCES.length];
    const former = FORMER_SOURCES[(bucket - 5 + FORMER_SOURCES.length) % FORMER_SOURCES.length];
    const manual = bucket === 0;
    const distilled = bucket === 2 || bucket === 7;
    return {
      id: `stress-memory-${String(i).padStart(3, "0")}`,
      text: `Stress memory ${String(i).padStart(3, "0")}. Deterministic graph fixture.`,
      sourceKind: manual ? "manual" : distilled ? "distilled" : "agent",
      sourceId: manual ? null : bucket <= 4 ? current : former,
      createdAt: "2026-09-05T00:00:00.000Z",
      updatedAt: "2026-09-05T00:00:00.000Z",
    };
  });

  const edges = [];
  const seen = new Set();
  let edgeSeed = 1;
  const edgeRandom = () => {
    edgeSeed = (edgeSeed * 1664525 + 1013904223) >>> 0;
    return edgeSeed / 4294967296;
  };
  const add = (left, right) => {
    if (left === right) return;
    const a = Math.min(left, right);
    const b = Math.max(left, right);
    const key = `${a}:${b}`;
    if (seen.has(key)) return;
    seen.add(key);
    edges.push({
      a: nodes[a].id,
      b: nodes[b].id,
      rel: edges.length % 11 === 0 ? "wiki" : "related",
      ...(edges.length % 11 === 0 ? {} : { score: 0.72 }),
    });
  };
  while (edges.length < EDGE_COUNT) {
    add(Math.floor(edgeRandom() * NODE_COUNT), Math.floor(edgeRandom() * NODE_COUNT));
  }

  assert.equal(nodes.length, NODE_COUNT);
  assert.equal(edges.length, EDGE_COUNT);
  return { nodes, edges };
}

function findChrome() {
  const candidates = [
    process.env.PUPPETEER_EXECUTABLE_PATH,
    process.env.CHROME_PATH,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].filter(Boolean);
  return candidates.find((candidate) => fs.existsSync(candidate));
}

async function isUp() {
  return fetch(BASE, { signal: AbortSignal.timeout(500) })
    .then(() => true)
    .catch(() => false);
}

async function startServer() {
  assert.equal(await isUp(), false, `port ${PORT} is already serving another checkout`);
  const child = spawn(
    "pnpm",
    ["exec", "vite", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"],
    { cwd: ROOT, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"] },
  );
  let output = "";
  child.stdout.on("data", (chunk) => {
    output += chunk;
  });
  child.stderr.on("data", (chunk) => {
    output += chunk;
  });
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (await isUp()) return child;
    if (child.exitCode != null) throw new Error(`Vite exited ${child.exitCode}: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Vite did not start on ${BASE}: ${output}`);
}

function stopServer(child) {
  if (!child || child.exitCode != null) return;
  try {
    if (process.platform === "win32") child.kill("SIGTERM");
    else process.kill(-child.pid, "SIGTERM");
  } catch {
    child.kill("SIGTERM");
  }
}

async function clickDestination(page, label) {
  const clicked = await page.evaluate((text) => {
    const button = [...document.querySelectorAll("button")].find((node) =>
      node.textContent?.includes(text),
    );
    button?.click();
    return Boolean(button);
  }, label);
  assert.equal(clicked, true, `missing ${label} destination`);
}

async function graphState(page) {
  return page.evaluate(() => {
    const svg = document.querySelector("svg.touch-none");
    const root = svg?.querySelector(":scope > g");
    const svgRect = svg?.getBoundingClientRect();
    const nodeGroups = [...(root?.querySelectorAll(":scope > g") ?? [])];
    const circles = nodeGroups
      .map((group) => [...group.querySelectorAll(":scope > circle")].at(-1))
      .filter(Boolean);
    const circleRects = circles.map((circle) => circle.getBoundingClientRect());
    const finite = circles.every(
      (circle) =>
        Number.isFinite(Number(circle.getAttribute("cx"))) &&
        Number.isFinite(Number(circle.getAttribute("cy"))),
    );
    const centersInCanvas = circleRects.filter(
      (rect) =>
        svgRect &&
        (rect.left + rect.right) / 2 >= svgRect.left &&
        (rect.left + rect.right) / 2 <= svgRect.right &&
        (rect.top + rect.bottom) / 2 >= svgRect.top &&
        (rect.top + rect.bottom) / 2 <= svgRect.bottom,
    ).length;
    const panelTitle = [...document.querySelectorAll("span")].find(
      (node) => node.textContent === "Graph settings",
    );
    const panelCard = panelTitle?.parentElement?.parentElement;
    const panelOuter = panelCard?.parentElement;
    const panelRect = panelOuter?.getBoundingClientRect();
    const obscuredByPanel = circleRects.filter((rect) => {
      if (!panelRect) return false;
      const x = (rect.left + rect.right) / 2;
      const y = (rect.top + rect.bottom) / 2;
      return x >= panelRect.left && x <= panelRect.right && y >= panelRect.top && y <= panelRect.bottom;
    }).length;
    const xs = circles.map((circle) => Number(circle.getAttribute("cx")));
    const ys = circles.map((circle) => Number(circle.getAttribute("cy")));
    const transform = root?.getAttribute("transform") ?? "";
    const scale = Number(transform.match(/scale\(([^)]+)\)/)?.[1]);
    const groupsButton = [...(panelCard?.querySelectorAll("button") ?? [])].find(
      (node) => node.textContent?.trim() === "Groups",
    );
    const groupsSection = groupsButton?.parentElement;
    const groupLabels = [...(groupsSection?.querySelectorAll("span.text-text-body") ?? [])].map(
      (node) => node.textContent ?? "",
    );
    const sourceTitles = [...(groupsSection?.querySelectorAll("[title]") ?? [])]
      .map((node) => node.getAttribute("title"))
      .filter(Boolean);
    return {
      circleCount: circles.length,
      edgeCount: root?.querySelectorAll(":scope > line").length ?? 0,
      finite,
      centersInCanvas,
      obscuredByPanel,
      worldWidth: Math.max(...xs) - Math.min(...xs),
      worldHeight: Math.max(...ys) - Math.min(...ys),
      transform,
      scale,
      labelCount: root?.querySelectorAll(":scope > g > text").length ?? 0,
      groupLabels,
      sourceTitles,
      panel: panelOuter
        ? {
            top: panelRect.top,
            bottom: panelRect.bottom,
            clientHeight: panelOuter.clientHeight,
            scrollHeight: panelOuter.scrollHeight,
          }
        : null,
    };
  });
}

async function waitForStableGraph(page, predicate, message) {
  const deadline = Date.now() + 3_000;
  let previous = null;
  let stableSamples = 0;
  let state = null;
  while (Date.now() < deadline) {
    state = await graphState(page);
    const stable =
      previous &&
      state.transform === previous.transform &&
      Math.abs(state.worldWidth - previous.worldWidth) <= 1.5 &&
      Math.abs(state.worldHeight - previous.worldHeight) <= 1.5;
    stableSamples = predicate(state) && stable ? stableSamples + 1 : 0;
    if (stableSamples >= 2) return state;
    previous = state;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.fail(`${message}; last state: ${JSON.stringify(state)}`);
}

function assertInitialState(state) {
  assert.equal(state.circleCount, NODE_COUNT, "every stress node must render");
  assert.equal(state.edgeCount, EDGE_COUNT, "every stress edge must render");
  assert.equal(state.finite, true, "node positions must remain finite");
  assert.equal(state.centersInCanvas, NODE_COUNT, "every stress node center must be visible after fit");
  assert.equal(state.obscuredByPanel, 0, "fit must keep stress nodes out from under settings");
  assert.ok(state.scale > 0.4, `fit remained pinned to its floor: ${state.transform}`);
  assert.ok(state.worldWidth / state.worldHeight < 3, "stress layout must not collapse horizontally");
  assert.equal(state.labelCount, 0, "dense overview must not render the global label cloud");
  assert.equal(state.groupLabels.filter((label) => label === "Shared").length, 1);
  assert.ok(state.groupLabels.some((label) => label.startsWith("Former agent · ")));
  assert.equal(
    state.groupLabels.some((label) => label.startsWith("Former agent · fx-ag-")),
    false,
    "distilled memories from current agents must resolve through the roster",
  );
  assert.ok(new Set(state.sourceTitles).size === state.sourceTitles.length, "source tooltips must be unique");
  assert.ok(state.panel && state.panel.bottom <= VIEWPORT.height - 16 + 1, "settings panel must fit viewport");
  assert.ok(state.panel && state.panel.scrollHeight > state.panel.clientHeight, "stress panel must scroll");
}

const graph = stressGraph();
const chrome = findChrome();
assert.ok(chrome, "Chrome or Chromium is required");
fs.mkdirSync(SHOTS, { recursive: true });

let server;
let browser;
try {
  server = await startServer();
  browser = await puppeteer.launch({
    executablePath: chrome,
    headless: true,
    args: ["--no-sandbox", "--hide-scrollbars", "--force-color-profile=srgb", "--disable-gpu"],
  });
  const page = await browser.newPage();
  await page.setViewport(VIEWPORT);
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error" || message.text().includes("[fixture]")) {
      consoleErrors.push(`${message.type()}: ${message.text()}`);
    }
  });
  page.on("pageerror", (error) => consoleErrors.push(`pageerror: ${error.message}`));

  await page.goto(`${BASE}/?fixture=default#view=memory`, { waitUntil: "load", timeout: 20_000 });
  await page.waitForSelector('body[data-conclave-ready="1"]', { timeout: 20_000 });
  await page.evaluate(async (payload) => {
    const data = await import("/src/fixtures/scenarios/data.ts");
    data.memoryGraph.nodes.splice(0, data.memoryGraph.nodes.length, ...payload.nodes);
    data.memoryGraph.edges.splice(0, data.memoryGraph.edges.length, ...payload.edges);
  }, graph);
  await clickDestination(page, "Blackboard");
  await new Promise((resolve) => setTimeout(resolve, 50));
  await clickDestination(page, "Memory");
  await page.waitForFunction(
    (count) => [...document.querySelectorAll("span")].some((node) => node.textContent === `${count} memories`),
    { timeout: 20_000 },
    NODE_COUNT,
  );
  await new Promise((resolve) => setTimeout(resolve, 600));
  await page.evaluate(() => document.documentElement.classList.add("dark"));

  const initial = await graphState(page);
  await page.screenshot({ path: path.join(SHOTS, "memory-stress-default.png"), type: "png" });
  console.log(JSON.stringify({ phase: "initial", ...initial }, null, 2));
  assertInitialState(initial);

  const firstCircle = await page.$("svg.touch-none > g > g > circle:last-of-type");
  assert.ok(firstCircle, "missing first graph node");
  const firstRect = await firstCircle.boundingBox();
  assert.ok(firstRect, "first graph node has no bounds");
  await page.mouse.move(firstRect.x + firstRect.width / 2, firstRect.y + firstRect.height / 2);
  await new Promise((resolve) => setTimeout(resolve, 80));
  const hover = await graphState(page);
  assert.ok(hover.labelCount >= 1 && hover.labelCount <= 10, "hover must label only focus and neighbours");
  await page.mouse.click(firstRect.x + firstRect.width / 2, firstRect.y + firstRect.height / 2);
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.equal(
    await page.evaluate(() => [...document.querySelectorAll("div")].some((node) => node.textContent?.includes("Stress memory 000."))),
    true,
    "selecting a node must open its detail",
  );

  const dragCircle = await page.$("svg.touch-none > g > g > circle:last-of-type");
  const dragRect = await dragCircle?.boundingBox();
  assert.ok(dragRect, "selected graph node has no drag bounds");
  const beforeDrag = await dragCircle.evaluate((circle) => ({
    x: Number(circle.getAttribute("cx")),
    y: Number(circle.getAttribute("cy")),
  }));
  await page.mouse.move(dragRect.x + dragRect.width / 2, dragRect.y + dragRect.height / 2);
  await page.mouse.down();
  await new Promise((resolve) => setTimeout(resolve, 20));
  await page.mouse.move(dragRect.x + dragRect.width / 2 + 28, dragRect.y + dragRect.height / 2 + 18, {
    steps: 6,
  });
  await new Promise((resolve) => setTimeout(resolve, 20));
  await page.mouse.up();
  const dragDeadline = Date.now() + 1_000;
  let afterDrag = beforeDrag;
  while (Date.now() < dragDeadline) {
    afterDrag = await dragCircle.evaluate((circle) => ({
      x: Number(circle.getAttribute("cx")),
      y: Number(circle.getAttribute("cy")),
    }));
    if (Math.hypot(afterDrag.x - beforeDrag.x, afterDrag.y - beforeDrag.y) > 5) break;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.ok(
    Math.hypot(afterDrag.x - beforeDrag.x, afterDrag.y - beforeDrag.y) > 5,
    "node drag must move the selected node",
  );

  await page.mouse.move(1350, 820);
  await page.mouse.click(1350, 820);
  assert.equal(
    await page.evaluate(() =>
      [...document.querySelectorAll("div")].some((node) => node.textContent?.includes("Stress memory 000.")),
    ),
    false,
    "background click must clear selection",
  );
  const search = await page.$('input[placeholder="Search memories…"]');
  assert.ok(search, "missing memory search");
  await search.type("Stress memory 000");
  await new Promise((resolve) => setTimeout(resolve, 80));
  const searched = await graphState(page);
  assert.equal(searched.labelCount, 1, "dense search must reveal only its matching label");
  await page.screenshot({ path: path.join(SHOTS, "memory-stress-focus-search.png"), type: "png" });
  await page.click('button[aria-label="Clear search"]');

  const beforeZoom = await graphState(page);
  await page.mouse.move(1200, 450);
  await page.mouse.wheel({ deltaY: -160 });
  await new Promise((resolve) => setTimeout(resolve, 80));
  const afterZoom = await graphState(page);
  assert.ok(afterZoom.scale > beforeZoom.scale, "wheel must zoom the graph");
  await page.mouse.move(1250, 780);
  await page.mouse.down();
  await page.mouse.move(1300, 800, { steps: 3 });
  await page.mouse.up();
  await new Promise((resolve) => setTimeout(resolve, 80));
  const afterPan = await graphState(page);
  assert.notEqual(afterPan.transform, afterZoom.transform, "background drag must pan the graph");

  await page.setViewport({ width: 1100, height: 700, deviceScaleFactor: 1 });
  await page.click('button[title="Fit graph to view"]');
  const constrained = await waitForStableGraph(
    page,
    (state) => state.centersInCanvas === NODE_COUNT,
    "resize and Fit did not settle with every node center visible",
  );
  assert.equal(constrained.centersInCanvas, NODE_COUNT, "resize and Fit must keep every node center visible");
  assert.ok(constrained.panel && constrained.panel.bottom <= 700 - 16 + 1, "panel must fit constrained height");
  await page.evaluate(() => {
    const title = [...document.querySelectorAll("span")].find((node) => node.textContent === "Graph settings");
    const outer = title?.parentElement?.parentElement?.parentElement;
    if (outer) outer.scrollTop = outer.scrollHeight;
  });
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.equal(
    await page.evaluate(() => {
      const label = [...document.querySelectorAll("span")].find((node) => node.textContent === "Link distance");
      if (!label) return false;
      const rect = label.getBoundingClientRect();
      return rect.top >= 0 && rect.bottom <= innerHeight;
    }),
    true,
    "lower force controls must be reachable by panel scroll",
  );
  await page.screenshot({ path: path.join(SHOTS, "memory-stress-constrained.png"), type: "png" });

  assert.deepEqual(consoleErrors, [], `browser console errors:\n${consoleErrors.slice(0, 8).join("\n")}`);
  console.log(
    `[memory-stress] PASS ${NODE_COUNT} nodes/${EDGE_COUNT} edges; screenshots in ${SHOTS}`,
  );
} finally {
  await browser?.close();
  stopServer(server);
}
