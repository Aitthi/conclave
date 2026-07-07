import fs from "node:fs";
import path from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";

// Spec D3: `design/public/` served at /p/<projectId>/<path> — the absolute-URL
// asset model that mirrors a real project's public/ dir. Traversal-guarded,
// no-store (design iteration, not production), 404 on unknown id or file.

const MIME: Record<string, string> = {
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  ".gif": "image/gif", ".svg": "image/svg+xml", ".webp": "image/webp",
  ".avif": "image/avif", ".ico": "image/x-icon",
  ".woff2": "font/woff2", ".woff": "font/woff", ".ttf": "font/ttf", ".otf": "font/otf",
  ".css": "text/css", ".js": "text/javascript", ".json": "application/json",
  ".txt": "text/plain", ".pdf": "application/pdf",
  ".mp4": "video/mp4", ".webm": "video/webm",
};

export function contentTypeFor(file: string): string {
  return MIME[path.extname(file).toLowerCase()] ?? "application/octet-stream";
}

// null = empty or escapes design/public (decoded before resolving, so an
// encoded ../ cannot sneak past the prefix check).
export function resolvePublicPath(designDir: string, rest: string): string | null {
  if (!rest) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(rest);
  } catch {
    return null;
  }
  const root = path.resolve(designDir, "public");
  const target = path.resolve(root, decoded);
  if (!target.startsWith(root + path.sep)) return null;
  return target;
}

// resolveProjectDir is injected (not imported) so this module stays pure —
// only node:fs/node:path — and loadable via the esbuild data-URL test pattern
// (a static top-level import of "./projects" has no base URL to resolve
// against from a data: URL and breaks module load entirely, not just the test).
export function publicAssets(resolveProjectDir: (id: string) => string | null) {
  return (req: IncomingMessage, res: ServerResponse, next: () => void) => {
    const url = (req.url || "").split("?")[0];
    const m = /^\/p\/([a-z0-9]+)\/(.*)$/.exec(url);
    if (!m) return next();
    const dir = resolveProjectDir(m[1]);
    const target = dir ? resolvePublicPath(dir, m[2]) : null;
    if (dir && target === null && m[2]) {
      res.statusCode = 403;
      res.end("forbidden");
      return;
    }
    if (!dir || !target || !fs.existsSync(target) || !fs.statSync(target).isFile()) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    res.statusCode = 200;
    res.setHeader("Content-Type", contentTypeFor(target));
    res.setHeader("Cache-Control", "no-store");
    fs.createReadStream(target).pipe(res);
  };
}
