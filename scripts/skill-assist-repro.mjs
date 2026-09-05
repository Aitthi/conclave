#!/usr/bin/env node
// skill-assist-repro — drive the REAL SkillEditor + SkillAssistPanel through the
// fixture IPC seam and assert the behaviours task skill-assist-repair restores.
//
// Written to FAIL against the pre-repair components (task skill-assist-diagnosis
// D1/D2/D3), so each assertion below names a defect that was reproduced first:
//   R1  keystrokes typed into the terminal reach the PTY  (was: disableStdin)
//   R2  the PTY is resized to the pane's real grid         (was: never resized)
//   R3  composer send = bracketed paste THEN standalone CR (was: paste only)
//   R4  manual Sync now; failed sync keeps last good values and surfaces an
//       error; Stop syncs before the destructive stop; a failed stop keeps the
//       draft and stays locked
//
// Runs in the same painting headless Chrome scripts/uishot.mjs uses: the
// conclave in-app browser throttles requestAnimationFrame when it is not
// foreground, which stalls xterm's renderer and FitAddon's measurement.
//
// Usage: node scripts/skill-assist-repro.mjs [--keep]   (needs vite on :1420)
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

// Default :1420 like uishot, but overridable: a peer's vite may already hold
// 1420 (vite.config.ts sets strictPort), and pointing this harness at another
// port beats killing someone else's server.
const portArg = process.argv.indexOf("--port");
const PORT = portArg >= 0 ? process.argv[portArg + 1] : "1420";
const BASE = `http://localhost:${PORT}`;
// NOTE: there is deliberately no `#view=` hash for the SKILL library —
// `#view=library` opens the AGENT Library (AppShell.tsx:805). The skill editor
// is reached the way a user reaches it: the rail's "Skill Library" button.
// Routing it via the hash would mean editing AppShell.tsx, outside this
// lane's boundary, and would test a path no user takes.
const URL_ = `${BASE}/?fixture=default#view=home`;
const require = createRequire(path.join(process.cwd(), "package.json"));
const puppeteer = require("puppeteer-core");

function findChrome() {
  const cands = [
    process.env.PUPPETEER_EXECUTABLE_PATH,
    process.env.CHROME_PATH,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ];
  const hit = cands.find((p) => p && fs.existsSync(p));
  if (!hit) throw new Error("no Chromium-family browser found");
  return hit;
}

// --shots also writes .shots/skill-editor-<state>.png for the pixel gate: a
// plain `uishot library` capture never opens the editor, so it cannot show the
// states this lane changed.
const SHOTS = process.argv.includes("--shots");
const shotDir = ".shots";
const shots = [];

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "  ok  " : "  FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};

const browser = await puppeteer.launch({
  executablePath: findChrome(),
  headless: "new",
  args: ["--no-sandbox"],
  defaultViewport: { width: 1440, height: 900, deviceScaleFactor: 2 },
});

try {
  const page = await browser.newPage();
  const consoleErrors = [];
  page.on("console", (m) => {
    const t = m.text();
    // A missing fixture handler throws loudly by design — never swallow it.
    if (m.type() === "error" || t.includes("[fixture]")) consoleErrors.push(t);
  });
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));

  const shoot = async (state) => {
    if (!SHOTS) return;
    fs.mkdirSync(shotDir, { recursive: true });
    const out = path.join(shotDir, `skill-editor-${state}.png`);
    await page.screenshot({ path: out });
    shots.push(path.resolve(out));
    console.log(`  shot  ${out}`);
  };

  await page.goto(URL_, { waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-conclave-ready="1"]', { timeout: 20000 });

  // Refuse to run against ANOTHER checkout's dev server. Several worktrees run
  // vite at once here and vite.config.ts sets strictPort, so a lane whose own
  // server failed to bind silently reuses a peer's and "verifies" their code
  // (CLAUDE.md records three such incidents). Vite serves this file from the
  // checkout it was started in, so asking it for a file only THIS worktree has
  // is a direct identity check.
  const marker = `/scripts/skill-assist-repro.mjs?identity=${Date.now()}`;
  const served = await page.evaluate(async (u) => {
    try {
      const r = await fetch(u);
      return r.ok ? (await r.text()).slice(0, 4000) : `HTTP ${r.status}`;
    } catch (e) { return `fetch failed: ${e.message}`; }
  }, BASE + marker).catch(() => null);
  if (!served || !served.includes("skill-assist-repro")) {
    throw new Error(
      `the dev server on ${BASE} is NOT this worktree's (it does not serve scripts/skill-assist-repro.mjs). ` +
      `Start this lane's server and pass --port, or check: lsof -nP -iTCP:${PORT} -sTCP:LISTEN`,
    );
  }


  // Only the DRAFT session's calls. The app's main Terminal.tsx is mounted
  // behind this overlay and resizes its own PTY; counting those would let the
  // assist panel "pass" R2 on another component's work (it did, at 101x46,
  // before this filter).
  const DRAFT_SESSION = "fx-skill-draft-session";
  const probe = async () =>
    (await page.evaluate(() => globalThis.skillAssistProbe.calls)).filter(
      (c) => c.sessionId === undefined || c.sessionId === DRAFT_SESSION,
    );
  const reset = () => page.evaluate(() => globalThis.skillAssistProbe.reset());
  const clickText = async (sel, text) => {
    const handle = await page.evaluateHandle(
      (s, t) => [...document.querySelectorAll(s)].find((e) => e.textContent.trim() === t),
      sel, text,
    );
    const el = handle.asElement();
    if (!el) throw new Error(`no ${sel} with text ${JSON.stringify(text)}`);
    await el.click();
  };

  // ── open the editor and start an assist session ────────────────────────
  await page.click('button[title="Skill Library"]');
  await page.waitForFunction(
    () => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === "New skill"),
    { timeout: 5000 },
  );
  await clickText("button", "New skill");
  await page.waitForSelector("textarea, input[placeholder='e.g. Code Reviewer']", { timeout: 5000 });
  await shoot("idle");
  await reset();
  await clickText("button", "Start");
  await page.waitForFunction(
    () => globalThis.skillAssistProbe.calls.some((c) => c.cmd === "skill.startDraftSession"),
    { timeout: 5000 },
  );
  // The terminal only exists once the session is live.
  await page.waitForSelector(".xterm", { timeout: 5000 });
  await page.waitForFunction(() => document.querySelector(".xterm-screen") !== null, { timeout: 5000 });

  // ── R2: the PTY must be resized to the pane's real grid ────────────────
  await page.waitForFunction(
    () => globalThis.skillAssistProbe.calls.some(
      (c) => c.cmd === "session.resize" && c.sessionId === "fx-skill-draft-session"),
    { timeout: 5000 },
  ).catch(() => {});
  await shoot("active");
  const resizes = (await probe()).filter((c) => c.cmd === "session.resize");
  check("R2 PTY resized at least once after mount", resizes.length > 0,
        `${resizes.length} session.resize calls`);
  const positive = resizes.filter((c) => c.cols > 0 && c.rows > 0);
  check("R2 every resize carries positive cols/rows", resizes.length > 0 && positive.length === resizes.length,
        JSON.stringify(resizes.map((c) => `${c.cols}x${c.rows}`)));
  // The pane is ~360px wide: a grid anywhere near the 80-col PTY default means
  // the fit never happened. This is the assertion that fails pre-repair.
  check("R2 grid matches the narrow pane, not the 80x24 PTY default",
        positive.length > 0 && positive.every((c) => c.cols < 80),
        JSON.stringify(positive.map((c) => `${c.cols}x${c.rows}`)));
  check("R2 resize reports pixel dims", positive.length > 0 && positive.every((c) => c.pixelWidth > 0 && c.pixelHeight > 0),
        JSON.stringify(positive.map((c) => `${c.pixelWidth}x${c.pixelHeight}`)));

  // ── R1: keystrokes typed at the terminal must reach the PTY ────────────
  // This is what makes the trust prompt answerable: arrows to move the
  // selection, Enter to confirm. Nothing is auto-trusted on the user's behalf.
  await reset();
  // xterm reads the keyboard through a hidden helper textarea — focus THAT,
  // not the screen div, or the presses go to the document and prove nothing.
  // Focus the ASSIST PANEL's terminal specifically. The app's main Terminal.tsx
  // is mounted behind this overlay and its .xterm-helper-textarea comes FIRST in
  // DOM order, so a bare querySelector focuses the wrong terminal — the presses
  // then land on the main session (observed: sessionId fx-sess-fx-ag-detoro) and
  // the assist panel looks broken when it is not.
  await page.evaluate(() => {
    const panel = [...document.querySelectorAll("div")]
      .filter((d) => d.querySelector(".xterm-helper-textarea") && /Ask agent to help/.test(d.textContent))
      .pop();
    if (!panel) throw new Error("assist panel terminal not found");
    panel.querySelector(".xterm-helper-textarea").focus();
  });
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.waitForFunction(
    () => globalThis.skillAssistProbe.calls.filter(
      (c) => c.cmd === "message.send" && c.sessionId === "fx-skill-draft-session").length >= 2,
    { timeout: 5000 },
  ).catch(() => {});
  const keys = (await probe()).filter((c) => c.cmd === "message.send");
  check("R1 arrow key reaches the PTY", keys.some((c) => c.text === "\x1b[B"),
        JSON.stringify(keys.map((c) => c.text)));
  check("R1 Enter reaches the PTY as CR", keys.some((c) => c.text === "\r"),
        JSON.stringify(keys.map((c) => c.text)));
  check("R1 keystrokes are raw, never bracketed pastes", keys.length > 0 && keys.every((c) => c.paste === false));
  check("R1 keystroke order preserved", (() => {
    const i = keys.findIndex((c) => c.text === "\x1b[B");
    const j = keys.findIndex((c) => c.text === "\r");
    return i >= 0 && j > i;
  })());
  check("R1 a visible hint tells the user the terminal is interactive",
        await page.evaluate(() => /click|type/i.test(document.body.innerText) &&
          /terminal|answer|question|prompt/i.test(document.body.innerText)));

  // ── R3: composer send = ONE bracketed paste, then a standalone CR ──────
  await reset();
  const long = "L".repeat(1500) + "\nsecond line";   // >1022 bytes AND multiline
  await page.type("textarea[placeholder='Message the agent…']", "x");
  await page.evaluate((text) => {
    const ta = document.querySelector("textarea[placeholder='Message the agent…']");
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value").set;
    setter.call(ta, text);
    ta.dispatchEvent(new Event("input", { bubbles: true }));
  }, long);
  await clickText("button", "") .catch(() => {});
  await page.evaluate(() => {
    const btns = [...document.querySelectorAll("button")];
    // the send button is the one next to the composer textarea
    const ta = document.querySelector("textarea[placeholder='Message the agent…']");
    const send = btns.find((b) => ta.parentElement.contains(b));
    send.click();
  });
  await page.waitForFunction(
    () => globalThis.skillAssistProbe.calls.filter(
      (c) => c.cmd === "message.send" && c.sessionId === "fx-skill-draft-session").length >= 2,
    { timeout: 5000 },
  ).catch(() => {});
  const sends = (await probe()).filter((c) => c.cmd === "message.send");
  check("R3 text sent as ONE bracketed paste, whole", sends.some((c) => c.paste === true && c.text === long),
        `${sends.length} sends, paste texts ${JSON.stringify(sends.filter(c=>c.paste).map((c) => c.text.length))}`);
  check("R3 a standalone CR follows the paste", (() => {
    const p = sends.findIndex((c) => c.paste === true);
    const cr = sends.findIndex((c, i) => i > p && c.text === "\r" && c.paste === false);
    return p >= 0 && cr > p;
  })(), JSON.stringify(sends.map((c) => (c.paste ? `paste(${c.text.length})` : JSON.stringify(c.text)))));
  check("R3 submits exactly once", sends.filter((c) => c.text === "\r").length === 1);
  check("R3 composer cleared after both succeed",
        await page.evaluate(() => document.querySelector("textarea[placeholder='Message the agent…']").value === ""));

  // ── R4: manual Sync now, and failure handling ─────────────────────────
  const hasSync = await page.evaluate(() =>
    document.querySelector('[aria-label="Sync now"]') !== null);
  check("R4 a manual Sync now control exists", hasSync);

  if (hasSync) {
    // successful sync applies the agent's edits to the editor fields
    await page.evaluate(() => globalThis.skillAssistProbe.setDraftFile({
      name: "Reviewed", description: "from the agent", content: "# body",
    }));
    await reset();
    await page.evaluate(() =>
      document.querySelector('[aria-label="Sync now"]').click());
    await page.waitForFunction(
      () => document.querySelector("input[placeholder='e.g. Code Reviewer']").value === "Reviewed",
      { timeout: 5000 },
    ).catch(() => {});
    await shoot("synced");
    // CONTENT included deliberately: asserting only name/description let a sync
    // that never applied the body pass (Armin, R5 coverage clarification).
    check("R4 successful sync applies name/description/content",
          await page.evaluate(() =>
            document.querySelector("input[placeholder='e.g. Code Reviewer']").value === "Reviewed" &&
            document.querySelector("input[placeholder='Shown in the Skill Library list']").value === "from the agent" &&
            document.querySelector(".cm-content").innerText.replace(/\u00a0/g, " ").trim() === "# body"));

    // a failing sync must preserve the last good values AND say so
    await page.evaluate(() => globalThis.skillAssistProbe.fail("sync", true));
    await page.evaluate(() =>
      document.querySelector('[aria-label="Sync now"]').click());
    await new Promise((r) => setTimeout(r, 300));
    check("R4 failed sync preserves the last good values",
          await page.evaluate(() =>
            document.querySelector("input[placeholder='e.g. Code Reviewer']").value === "Reviewed"));
    check("R4 failed sync surfaces an actionable error",
          await page.evaluate(() => /sync/i.test(document.body.innerText) && /fail|error|retry|could ?n[o']t/i.test(document.body.innerText)));
    await page.evaluate(() => globalThis.skillAssistProbe.fail("sync", false));

    // Stop must sync BEFORE the destructive stop, so the newest file is not lost
    await page.evaluate(() => globalThis.skillAssistProbe.setDraftFile({
      name: "Final", description: "last words", content: "# final",
    }));
    await reset();
    await page.evaluate(() => globalThis.skillAssistProbe.fail("stop", true));
    await page.evaluate(() =>
      document.querySelector("[aria-label='Stop agent']").click());
    await new Promise((r) => setTimeout(r, 400));
    const stopCalls = await probe();
    const si = stopCalls.findIndex((c) => c.cmd === "skill.syncDraft");
    const ti = stopCalls.findIndex((c) => c.cmd === "skill.stopDraftSession");
    check("R4 Stop syncs before the destructive stop", si >= 0 && ti > si,
          JSON.stringify(stopCalls.map((c) => c.cmd)));
    check("R4 a failed stop keeps the draft and stays locked",
          await page.evaluate(() =>
            document.querySelector("[aria-label='Stop agent']") !== null &&
            document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === true));
    check("R4 a failed stop surfaces a retryable error",
          await page.evaluate(() => /stop/i.test(document.body.innerText) && /fail|error|retry|again/i.test(document.body.innerText)));

    // a successful stop unlocks the editor
    await page.evaluate(() => globalThis.skillAssistProbe.fail("stop", false));
    await page.evaluate(() => document.querySelector("[aria-label='Stop agent']").click());
    await page.waitForFunction(
      () => document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === false,
      { timeout: 5000 },
    ).catch(() => {});
    check("R4 a successful stop unlocks the editor",
          await page.evaluate(() =>
            document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === false));
    check("R4 the last sync's values survived into the editor",
          await page.evaluate(() =>
            document.querySelector("input[placeholder='e.g. Code Reviewer']").value === "Final"));
  }

  // ── R4 amendment (challenge 9f95a320): PERMANENT sync failure ─────────
  // read_draft returns None for a missing/unparseable SKILL.md and sync then
  // fails forever, so a strict sync-before-stop gate would wedge the session.
  const startSession = async () => {
    await clickText("button", "Start");
    await page.waitForSelector(".xterm", { timeout: 5000 });
    await page.waitForFunction(() => document.querySelector("[aria-label='Stop agent']") !== null,
      { timeout: 5000 });
  };
  const nameValue = () =>
    page.evaluate(() => document.querySelector("input[placeholder='e.g. Code Reviewer']").value);
  const locked = () =>
    page.evaluate(() => document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === true);

  await startSession();
  // The editor shows "Final" from the run above; the unreadable draft differs.
  const shownBefore = await nameValue();
  await page.evaluate(() => {
    globalThis.skillAssistProbe.setDraftFile({ name: "NeverReadable", content: "x" });
    globalThis.skillAssistProbe.fail("sync", true);      // permanent
    globalThis.skillAssistProbe.fail("stop", true);      // first fallback attempt fails
  });
  await reset();
  await page.evaluate(() => document.querySelector("[aria-label='Stop agent']").click());
  await new Promise((r) => setTimeout(r, 400));
  const firstStop = await probe();
  check("R4amend permanent sync failure: first Stop keeps the session",
        (await locked()) === true && firstStop.every((c) => c.cmd !== "skill.stopDraftSession"),
        JSON.stringify(firstStop.map((c) => c.cmd)));
  await shoot("sync-failed-fallback");
  check("R4amend the fallback is offered with the ruled copy",
        await page.evaluate(() =>
          /latest draft could not be read/i.test(document.body.innerText) &&
          /keeps the version shown in the editor and discards unsynced draft changes/i.test(document.body.innerText) &&
          /Retry Sync remains available/i.test(document.body.innerText)));

  // failed fallback stop stays retryable
  await reset();
  await clickText("button", "Stop without syncing");
  await new Promise((r) => setTimeout(r, 400));
  check("R4amend a failed fallback stop remains retryable",
        (await locked()) === true &&
        (await page.evaluate(() =>
          document.body.innerText.includes("Stop without syncing") && /try again|retry/i.test(document.body.innerText))));

  // succeeding fallback stops, unlocks, and keeps the DISPLAYED version
  await page.evaluate(() => globalThis.skillAssistProbe.fail("stop", false));
  await clickText("button", "Stop without syncing");
  await page.waitForFunction(
    () => document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === false,
    { timeout: 5000 },
  ).catch(() => {});
  check("R4amend explicit fallback stops and unlocks", (await locked()) === false);
  check("R4amend the displayed version is preserved, not the unreadable draft",
        (await nameValue()) === shownBefore,
        `shown=${await nameValue()} expected=${shownBefore}`);
  check("R4amend the fallback UI is gone once stopped",
        await page.evaluate(() => !/latest draft could not be read/i.test(document.body.innerText)));
  await page.evaluate(() => globalThis.skillAssistProbe.fail("sync", false));

  // ── R4: closing the editor mid-start must not leak a hidden session ────
  await page.evaluate(() => globalThis.skillAssistProbe.setStartDelay(600));
  await reset();
  await clickText("button", "Start");
  await new Promise((r) => setTimeout(r, 120));      // still in flight
  await clickText("button", "Cancel");               // close the editor
  await new Promise((r) => setTimeout(r, 1200));     // let the start resolve
  const afterClose = await probe();
  check("R4 a start that resolves after close is stopped, not leaked",
        afterClose.some((c) => c.cmd === "skill.stopDraftSession"),
        JSON.stringify(afterClose.map((c) => c.cmd)));
  await page.evaluate(() => globalThis.skillAssistProbe.setStartDelay(0));

  // ── R5 (Armin finding 49e88209): same-session sync ORDERING ───────────
  // R4's guard only compares workspaceAgentId, so two reads inside ONE session
  // both pass it. An older idle sync resolving after Stop's final sync
  // overwrites the final content — and the stop then deletes the scratch dir,
  // so what the user keeps is the STALE read. Response order is controlled
  // here; nothing about it is timing-luck.
  const contentValue = () =>
    page.evaluate(() => document.querySelector(".cm-content").innerText.replace(/\u00a0/g, " ").trim());

  // The leak test above closed the editor; reopen a fresh one.
  const openNewEditor = async () => {
    if (await page.evaluate(() => document.querySelector("input[placeholder='e.g. Code Reviewer']") !== null)) return;
    await page.waitForFunction(
      () => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === "New skill"),
      { timeout: 5000 },
    );
    await clickText("button", "New skill");
    await page.waitForSelector("input[placeholder='e.g. Code Reviewer']", { timeout: 5000 });
  };
  await page.evaluate(() => globalThis.skillAssistProbe.resetSyncs());
  await openNewEditor();
  await startSession();
  await page.evaluate(() => {
    globalThis.skillAssistProbe.setSyncMode("manual");
    globalThis.skillAssistProbe.setDraftFile({ name: "OLD", description: "old desc", content: "# OLD BODY" });
  });
  await reset();
  // S1: a background idle sync captures the OLD file and is held open.
  await page.evaluate(() => globalThis.skillAssistProbe.emitStatus("idle"));
  await page.waitForFunction(() => globalThis.skillAssistProbe.pendingSyncIds().length === 1, { timeout: 5000 });
  const [s1] = await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds());

  // The agent then writes the FINAL file, and the user hits Stop.
  await page.evaluate(() => {
    globalThis.skillAssistProbe.setDraftFile({ name: "FINAL", description: "final desc", content: "# FINAL BODY" });
    globalThis.skillAssistProbe.fail("stop", true);   // stop pending/failing while S1 is still open
  });
  await page.evaluate(() => document.querySelector("[aria-label='Stop agent']").click());
  await page.waitForFunction(() => globalThis.skillAssistProbe.pendingSyncIds().length === 2, { timeout: 5000 });
  const ids = await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds());
  const s2 = ids.find((i) => i !== s1);

  // A Stop attempt OWNS its final sync: no new read may be started under it.
  await page.evaluate(() => globalThis.skillAssistProbe.emitStatus("idle"));
  await new Promise((r) => setTimeout(r, 200));
  check("R5 Stop suppresses new syncs while its final read is in flight",
        (await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds().length)) === 2,
        `pending=${JSON.stringify(await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds()))}`);

  // S2 (Stop's final read) resolves FIRST, then the older S1 resolves.
  await page.evaluate((id) => globalThis.skillAssistProbe.resolveSync(id), s2);
  await new Promise((r) => setTimeout(r, 250));
  await page.evaluate((id) => globalThis.skillAssistProbe.resolveSync(id), s1);
  await new Promise((r) => setTimeout(r, 350));

  check("R5 a stale in-session sync does not overwrite the final name",
        (await nameValue()) === "FINAL", `name=${await nameValue()}`);
  check("R5 a stale in-session sync does not overwrite the final description",
        (await page.evaluate(() =>
          document.querySelector("input[placeholder='Shown in the Skill Library list']").value)) === "final desc");
  check("R5 a stale in-session sync does not overwrite the final CONTENT",
        (await contentValue()) === "# FINAL BODY", `content=${JSON.stringify(await contentValue())}`);

  // A stale FAILURE after a newer success must not raise an error either.
  await page.evaluate(() => globalThis.skillAssistProbe.setSyncMode("manual"));
  await reset();
  await page.evaluate(() => document.querySelector('[aria-label="Sync now"]').click());
  await page.waitForFunction(() => globalThis.skillAssistProbe.pendingSyncIds().length === 1, { timeout: 5000 });
  const [sOld] = await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds());
  // The newer read must come from the BACKGROUND idle path: handleSyncNow has a
  // `syncing` guard, so a second button click while the first is in flight is
  // ignored and there would be no newer success to be stale against.
  await page.evaluate(() => globalThis.skillAssistProbe.setSyncMode("immediate"));
  await page.evaluate(() => globalThis.skillAssistProbe.emitStatus("idle")); // newer, succeeds now
  await new Promise((r) => setTimeout(r, 250));
  await page.evaluate((id) => globalThis.skillAssistProbe.rejectSync(id, "stale read failed"), sOld);
  await new Promise((r) => setTimeout(r, 300));
  check("R5 a stale sync FAILURE after a newer success shows no error",
        await page.evaluate(() => !/Couldn't read the agent's file/i.test(document.body.innerText)));
  check("R5 the newer success still stands after the stale failure",
        (await nameValue()) === "FINAL" && (await contentValue()) === "# FINAL BODY");

  // Retry the stop successfully: it must unlock and keep the final values.
  await page.evaluate(() => globalThis.skillAssistProbe.fail("stop", false));
  await page.evaluate(() => document.querySelector("[aria-label='Stop agent']").click());
  await page.waitForFunction(
    () => document.querySelector("input[placeholder='e.g. Code Reviewer']").disabled === false,
    { timeout: 5000 },
  ).catch(() => {});
  check("R5 a successful retry unlocks the editor", (await locked()) === false);
  check("R5 the final values survive the stop", (await nameValue()) === "FINAL" && (await contentValue()) === "# FINAL BODY");

  // ── Save -> close -> reopen: what the user actually keeps ──────────────
  await reset();
  await clickText("button", "Save skill");
  // Wait for the EDITOR to close, not for "New skill" to exist: that button is
  // in the Library BEHIND this overlay, so waiting on it returns instantly and
  // the assertions below then read state before the save has resolved.
  await page.waitForFunction(
    () => document.querySelector("input[placeholder='e.g. Code Reviewer']") === null,
    { timeout: 5000 },
  );
  const saved = (await probe()).filter((c) => c.cmd === "skill.save");
  check("R5 Save persists the FINAL content", saved.length === 1 && saved[0].name === "FINAL",
        JSON.stringify(saved));
  // Reopen it from the library and read the fields back.
  await page.evaluate(() => {
    const edit = [...document.querySelectorAll("button")].filter((b) => b.textContent.trim() === "Edit").pop();
    edit.click();
  });
  await page.waitForSelector("input[placeholder='e.g. Code Reviewer']", { timeout: 5000 });
  await new Promise((r) => setTimeout(r, 200));
  check("R5 reopened skill has the final name", (await nameValue()) === "FINAL", `name=${await nameValue()}`);
  check("R5 reopened skill has the final description",
        (await page.evaluate(() =>
          document.querySelector("input[placeholder='Shown in the Skill Library list']").value)) === "final desc");
  check("R5 reopened skill has the final CONTENT", (await contentValue()) === "# FINAL BODY",
        `content=${JSON.stringify(await contentValue())}`);
  await shoot("saved-reopened");

  // ── a sync completing after unmount must not apply ─────────────────────
  await page.evaluate(() => globalThis.skillAssistProbe.resetSyncs());
  await openNewEditor();
  await startSession();
  await page.evaluate(() => {
    globalThis.skillAssistProbe.setSyncMode("manual");
    globalThis.skillAssistProbe.setDraftFile({ name: "AFTER-UNMOUNT", description: "x", content: "# nope" });
  });
  await page.evaluate(() => document.querySelector('[aria-label="Sync now"]').click());
  await page.waitForFunction(() => globalThis.skillAssistProbe.pendingSyncIds().length === 1, { timeout: 5000 });
  const [sGhost] = await page.evaluate(() => globalThis.skillAssistProbe.pendingSyncIds());
  await clickText("button", "Cancel");                       // close the editor
  await new Promise((r) => setTimeout(r, 200));
  const ghostErrors = [];
  page.on("pageerror", (e) => ghostErrors.push(e.message));
  await page.evaluate((id) => globalThis.skillAssistProbe.resolveSync(id), sGhost);
  await new Promise((r) => setTimeout(r, 300));
  check("R5 a sync completing after unmount applies nothing and throws nothing",
        ghostErrors.length === 0 &&
        (await page.evaluate(() =>
          document.querySelector("input[placeholder='e.g. Code Reviewer']") === null)),
        ghostErrors.join(" | "));
  await page.evaluate(() => globalThis.skillAssistProbe.resetSyncs());

  check("no console errors or [fixture] misses", consoleErrors.length === 0,
        consoleErrors.slice(0, 3).join(" | "));

  if (SHOTS) {
    console.log("\nshots:");
    for (const p2 of shots) console.log(`  ${p2}`);
  }
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  if (failed.length) {
    console.log("FAILED:");
    for (const f of failed) console.log(`  - ${f.name}${f.detail ? ` — ${f.detail}` : ""}`);
  }
  await browser.close();
  process.exit(failed.length ? 1 : 0);
} catch (e) {
  console.error("HARNESS ERROR:", e.message);
  await browser.close();
  process.exit(2);
}
