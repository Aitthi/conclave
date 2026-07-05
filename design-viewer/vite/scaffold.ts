import fs from "node:fs";
import path from "node:path";

// Shared scaffold for a project's `.arta/proto/` starter — the single source both the
// launcher (bin/arta.mjs, via arta-watch.ts's configureServer) and the MCP `arta_doctor`
// tool (Task 15) use to bootstrap or repair a project's React canvas. `projectDir` is
// the PROJECT ROOT (the folder that CONTAINS `.arta`), NOT the `.arta` dir itself — the
// opposite convention from vite/projects.ts's resolveProjectDir/idFor, which take the
// `.arta` dir as "the project dir". Both are correct for their own contract; this
// function is a self-contained "given a project root, ensure .arta/proto/... exists"
// utility, unrelated to project-id resolution.
//
// Deliberately plain node:fs/node:path — no Vite-only APIs — so it works unmodified
// from arta_doctor's MCP process, which never runs under Vite.

const THEME_CSS = `@import "tailwindcss";
@source "./";
@custom-variant dark (&:where(.dark, .dark *));

@theme {
  --color-primary: oklch(0.66 0.2 250);
  --color-bg: #ffffff;
  --color-fg: #18181b;
  --font-sans: "Geist", "Noto Sans Thai", system-ui, sans-serif;
  --radius-lg: 1rem;
}

.dark {
  --color-bg: #0b0b0c;
  --color-fg: #fafafa;
}

body { background: var(--color-bg); color: var(--color-fg); font-family: var(--font-sans); }
`;

const HOME_TSX = `export const meta = { title: "Home" };

export default function Home() {
  return (
    <main className="min-h-screen grid place-items-center bg-bg text-fg">
      <div className="text-center space-y-3">
        <h1 className="text-4xl font-semibold">Your canvas is live</h1>
        <p className="opacity-70">Ask your agent to design something — screens are React files in .arta/proto/screens/.</p>
      </div>
    </main>
  );
}
`;

// Matches the scaffolded screens/home.tsx's id ("home") — the minimal default
// consistent with ProtoConfig.start.
const CONFIG_JSON = `{ "start": "home" }\n`;

export function scaffoldProto(projectDir: string): { created: string[] } {
  const root = path.resolve(projectDir);
  const protoDir = path.join(root, ".arta", "proto");
  const created: string[] = [];

  const ensureFile = (abs: string, contents: string) => {
    if (fs.existsSync(abs)) return;
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, contents);
    created.push(path.relative(root, abs));
  };
  const ensureDir = (abs: string) => {
    if (fs.existsSync(abs)) return;
    fs.mkdirSync(abs, { recursive: true });
    created.push(path.relative(root, abs));
  };

  ensureFile(path.join(protoDir, "config.json"), CONFIG_JSON);
  ensureFile(path.join(protoDir, "theme.css"), THEME_CSS);
  ensureFile(path.join(protoDir, "screens", "home.tsx"), HOME_TSX);
  ensureDir(path.join(protoDir, "lib"));
  ensureDir(path.join(protoDir, "components"));

  return { created };
}
