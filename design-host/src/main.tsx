import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Shell } from "./Shell";
import "./index.css";

const project = new URLSearchParams(location.search).get("project") ?? "";
// Spec D3: screens/lib build absolute asset URLs without hard-coding a project
// id — e.g. a workspace helper `asset(p) => `${window.__DESIGN_PUBLIC_BASE__}/${p}``.
(window as unknown as { __DESIGN_PUBLIC_BASE__: string }).__DESIGN_PUBLIC_BASE__ = `/p/${project}`;

// The project id is runtime data (from the URL query string), so this dynamic import
// must stay `/* @vite-ignore */`-marked opaque — the static analyzer must not try to
// resolve it (the manifest is a per-project virtual module, see vite/host-app.ts).
const mod = await import(
  /* @vite-ignore */ `/@id/virtual:design-host-manifest/${project}`
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Shell screens={mod.screens} screenIds={mod.screenIds} />
  </StrictMode>
);
