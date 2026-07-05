import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Shell } from "./Shell";
import "./index.css";

// The project id is runtime data (from the URL query string), so this dynamic import
// must stay `/* @vite-ignore */`-marked opaque — the static analyzer must not try to
// resolve it (the manifest is a per-project virtual module, see vite/host-app.ts).
const mod = await import(
  /* @vite-ignore */ `/@id/virtual:design-host-manifest/${new URLSearchParams(location.search).get("project") ?? ""}`
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Shell screens={mod.screens} screenIds={mod.screenIds} />
  </StrictMode>
);
