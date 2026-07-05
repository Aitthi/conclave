import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Shell } from "./shell/Shell";
import "./shell/base.css";

// In `vite dev`, the project id is runtime data (from the URL query string), so the
// import must stay a `/* @vite-ignore */`-marked opaque expression the static analyzer
// doesn't try to resolve. In a static export (Task 12's `exportStatic`), the build
// passes the target project id as a `define`d compile-time constant instead, and this
// branch builds the specifier with STRING CONCATENATION rather than a template
// literal — that distinction matters: esbuild's `define` substitution folds
// `"literal" + DEFINED_CONST` into a single plain string Literal AST node (verified by
// inspecting esbuild's transform output directly), but it does NOT rewrite a
// TemplateLiteral into a Literal node even when its only interpolation is itself a
// literal — and Rollup's dynamic-import module-graph tracing only treats a plain
// string Literal argument as statically resolvable. A template-literal version of this
// same branch (tried first) silently produced an untraced, unbundled import — Rollup
// never called this plugin's resolveId/load for it at all. With string concatenation,
// Rollup traces and bundles the manifest (and everything it imports — screens,
// theme.css) into the export. Without this branch, a `/* @vite-ignore */` import is
// never traced, and the exported bundle would ship a broken, unresolved runtime import
// string.
declare const __ARTA_EXPORT_PROJECT__: string | undefined;

const mod =
  typeof __ARTA_EXPORT_PROJECT__ !== "undefined"
    ? await import("/@id/virtual:arta-proto-manifest/" + __ARTA_EXPORT_PROJECT__)
    : await import(
        /* @vite-ignore */ `/@id/virtual:arta-proto-manifest/${new URLSearchParams(location.search).get("project") ?? "home"}`
      );
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Shell config={mod.config} metas={mod.metas} screens={mod.screens} components={mod.components} />
  </StrictMode>
);
