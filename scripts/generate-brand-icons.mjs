#!/usr/bin/env node

import { access, copyFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(scriptDir, "..");
const source = path.join(root, "public", "brand", "app-icon.svg");
const previewSource = path.join(root, "public", "brand", "brand-preview.svg");
const previewOutput = path.join(root, "public", "brand", "preview.png");
const outputDir = path.join(root, "src-tauri", "icons");
const tauriCli = process.env.TAURI_CLI || path.join(root, "node_modules", ".bin", "tauri");
const magick = process.env.MAGICK || "magick";

const iconFiles = [
  "32x32.png",
  "128x128.png",
  "128x128@2x.png",
  "icon.png",
  "icon.ico",
  "icon.icns",
  "Square30x30Logo.png",
  "Square44x44Logo.png",
  "Square71x71Logo.png",
  "Square89x89Logo.png",
  "Square107x107Logo.png",
  "Square142x142Logo.png",
  "Square150x150Logo.png",
  "Square284x284Logo.png",
  "Square310x310Logo.png",
  "StoreLogo.png",
];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8" });
  if (result.status !== 0) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n");
    throw new Error(`${command} ${args.join(" ")} failed\n${details}`);
  }
}

await access(source);
await access(previewSource);
await access(tauriCli).catch(() => {
  throw new Error(`Tauri CLI not found at ${tauriCli}. Run pnpm install or set TAURI_CLI.`);
});

const temporaryOutput = await mkdtemp(path.join(tmpdir(), "conclave-brand-icons-"));

try {
  run(tauriCli, ["icon", source, "--output", temporaryOutput]);
  await mkdir(outputDir, { recursive: true });
  await Promise.all(
    iconFiles.map((name) => copyFile(path.join(temporaryOutput, name), path.join(outputDir, name))),
  );
  run(magick, [
    "-background",
    "none",
    previewSource,
    "-resize",
    "1600x1000!",
    "-strip",
    "-depth",
    "8",
    previewOutput,
  ]);
} finally {
  await rm(temporaryOutput, { recursive: true, force: true });
}

console.log(`Generated ${iconFiles.length} desktop assets and ${path.relative(root, previewOutput)}.`);
