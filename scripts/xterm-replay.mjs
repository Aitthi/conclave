#!/usr/bin/env node

import fs from "node:fs/promises";
import process from "node:process";
import headless from "@xterm/headless";
import { Unicode11Addon } from "@xterm/addon-unicode11";

const { Terminal } = headless;


function usage(message) {
  if (message) console.error(message);
  console.error(
    "usage: node scripts/xterm-replay.mjs <rec.jsonl> " +
      "[--convert-eol true|false] [--unicode 6|11] [--dump-every-key] " +
      "[--start-size CxR] [--resize-at-ms N]",
  );
  process.exit(1);
}


function parseBoolean(value, flag) {
  if (value === "true") return true;
  if (value === "false") return false;
  usage(`${flag} must be true or false`);
}


function parseSize(value, flag) {
  const match = /^(\d+)[xX](\d+)$/.exec(value ?? "");
  if (!match) usage(`${flag} must be CxR, for example 80x24`);
  const cols = Number(match[1]);
  const rows = Number(match[2]);
  if (cols < 2 || rows < 1) usage(`${flag} must be at least 2x1`);
  return { cols, rows };
}


function parseMilliseconds(value, flag) {
  const milliseconds = Number(value);
  if (!Number.isInteger(milliseconds) || milliseconds < 0) {
    usage(`${flag} must be a non-negative integer`);
  }
  return milliseconds;
}


function parseArgs(argv) {
  if (argv.length === 0 || argv[0].startsWith("--")) usage();
  const options = {
    recording: argv[0],
    convertEol: true,
    unicode: "11",
    dumpEveryKey: false,
    startSize: null,
    resizeAtMs: null,
  };
  for (let index = 1; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--dump-every-key") {
      options.dumpEveryKey = true;
    } else if (arg === "--convert-eol") {
      options.convertEol = parseBoolean(argv[++index], arg);
    } else if (arg === "--unicode") {
      options.unicode = argv[++index];
      if (options.unicode !== "6" && options.unicode !== "11") {
        usage("--unicode must be 6 or 11");
      }
    } else if (arg === "--start-size") {
      options.startSize = parseSize(argv[++index], arg);
    } else if (arg === "--resize-at-ms") {
      options.resizeAtMs = parseMilliseconds(argv[++index], arg);
    } else {
      usage(`unknown argument: ${arg}`);
    }
  }
  if (options.resizeAtMs !== null && options.startSize === null) {
    usage("--resize-at-ms requires --start-size");
  }
  return options;
}


function decode(record) {
  return Buffer.from(record.b64, "base64");
}


function write(term, bytes) {
  return new Promise((resolve) => term.write(bytes, resolve));
}


function screen(term) {
  const buffer = term.buffer.normal;
  const first = Math.max(0, buffer.length - term.rows);
  const lines = [];
  for (let row = first; row < buffer.length; row += 1) {
    const line = buffer.getLine(row);
    lines.push({ row: row + 1, text: line?.translateToString(true) ?? "" });
  }
  return { buffer, lines };
}


function dumpScreen(term, label) {
  const { buffer, lines } = screen(term);
  console.log(`--- ${label} (${term.cols}x${term.rows}, normal buffer) ---`);
  for (const line of lines) {
    console.log(`${String(line.row).padStart(4)} |${line.text}`);
  }
  console.log(
    `cursor: x=${buffer.cursorX} y=${buffer.cursorY} ` +
      `absoluteRow=${buffer.baseY + buffer.cursorY + 1}`,
  );
}


function inspectLayout(term) {
  const { lines } = screen(term);
  const commandWithDescription = /\/[A-Za-z][A-Za-z0-9:_-]*\s{2,}\S/;
  const commandIndexes = [];
  for (let index = 0; index < lines.length; index += 1) {
    if (commandWithDescription.test(lines[index].text)) commandIndexes.push(index);
  }
  if (commandIndexes.length === 0) return { exercised: false, violations: [] };

  const first = commandIndexes[0];
  const last = commandIndexes.at(-1);
  const violations = [];
  for (let index = first; index <= last; index += 1) {
    const line = lines[index];
    if (!line.text.trim()) continue;
    const malformedCommand =
      commandWithDescription.test(line.text) && !line.text.startsWith("  /");
    const flushLeftListRow =
      !line.text.startsWith(" ") && /^[^\s─]+\s{2,}\S/.test(line.text);
    if (malformedCommand || flushLeftListRow) {
      violations.push(line);
    }
  }
  return { exercised: true, violations };
}


async function main() {
  const options = parseArgs(process.argv.slice(2));
  const contents = await fs.readFile(options.recording, "utf8");
  const records = contents
    .split("\n")
    .filter(Boolean)
    .map((line, index) => {
      try {
        return { ...JSON.parse(line), sequence: index };
      } catch (error) {
        throw new Error(`invalid JSON on line ${index + 1}: ${error.message}`);
      }
    })
    .sort((left, right) => left.t_ms - right.t_ms || left.sequence - right.sequence);

  const initialResize = records.find((record) => record.kind === "resize");
  if (options.resizeAtMs !== null && initialResize === undefined) {
    throw new Error("--resize-at-ms requires a resize record to supply the real size");
  }
  const term = new Terminal({
    cols: options.startSize?.cols ?? initialResize?.cols ?? 80,
    rows: options.startSize?.rows ?? initialResize?.rows ?? 24,
    convertEol: options.convertEol,
    scrollback: 12000,
    allowProposedApi: true,
  });
  if (options.unicode === "11") {
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
  }

  let pendingKey = null;
  let exercisedFrames = 0;
  const frameViolations = [];
  let injectedResize = false;
  for (const record of records) {
    if (
      options.resizeAtMs !== null &&
      !injectedResize &&
      record.t_ms >= options.resizeAtMs
    ) {
      term.resize(initialResize.cols, initialResize.rows);
      injectedResize = true;
    }
    if (record.kind === "key") {
      if (pendingKey !== null) {
        const label = `after key ${JSON.stringify(pendingKey)}`;
        if (options.dumpEveryKey) dumpScreen(term, label);
        const inspection = inspectLayout(term);
        if (inspection.exercised) exercisedFrames += 1;
        if (inspection.violations.length > 0) {
          frameViolations.push({ label, violations: inspection.violations });
        }
      }
      pendingKey = decode(record).toString("utf8");
    } else if (record.kind === "resize") {
      const isInitialResize = record.sequence === initialResize?.sequence;
      if (options.startSize === null || !isInitialResize) {
        term.resize(record.cols, record.rows);
      }
    } else if (record.kind === "out") {
      await write(term, decode(record));
    } else {
      throw new Error(`unknown record kind: ${record.kind}`);
    }
  }
  if (options.resizeAtMs !== null && !injectedResize) {
    term.resize(initialResize.cols, initialResize.rows);
  }
  if (pendingKey !== null) {
    const label = `after key ${JSON.stringify(pendingKey)}`;
    if (options.dumpEveryKey) dumpScreen(term, label);
    const inspection = inspectLayout(term);
    if (inspection.exercised) exercisedFrames += 1;
    if (inspection.violations.length > 0) {
      frameViolations.push({ label, violations: inspection.violations });
    }
  }

  dumpScreen(
    term,
    `final convertEol=${options.convertEol} unicode=${options.unicode}` +
      (options.startSize === null
        ? ""
        : ` startSize=${options.startSize.cols}x${options.startSize.rows}`) +
      (options.resizeAtMs === null ? "" : ` resizeAtMs=${options.resizeAtMs}`),
  );
  if (frameViolations.length > 0) {
    console.error("LAYOUT INVARIANT VIOLATION:");
    for (const frame of frameViolations) {
      console.error(frame.label);
      for (const violation of frame.violations) {
        console.error(`${String(violation.row).padStart(4)} |${violation.text}`);
      }
    }
    term.dispose();
    process.exitCode = 2;
    return;
  }
  if (exercisedFrames === 0) {
    console.error(
      "LAYOUT INVARIANT: NOT EXERCISED (no autocomplete block in any frame)",
    );
    term.dispose();
    process.exitCode = 3;
    return;
  }
  console.log("LAYOUT INVARIANT: PASS");
  term.dispose();
}


main().catch((error) => {
  console.error(error.stack ?? error.message);
  process.exitCode = 1;
});
