import { EditorView } from "@codemirror/view";

/**
 * Dark CodeMirror theme for the skill content editor, matching this app's
 * real dark palette (src/styles/app.css `.dark` block) exactly rather than
 * a generic third-party theme — the editor's colors must match the rest of
 * the (currently dark-only) full-panel Skill Editor, not clash with it.
 */
export const skillContentEditorTheme = EditorView.theme(
  {
    "&": {
      backgroundColor: "#1c1c1e",
      color: "#d8d8da",
      height: "100%",
    },
    ".cm-content": {
      caretColor: "#f5f5f7",
      fontFamily: '"SF Mono", "JetBrains Mono", ui-monospace, Menlo, monospace',
      fontSize: "12.5px",
    },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#f5f5f7" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
      backgroundColor: "rgba(10, 132, 255, 0.25)",
    },
    ".cm-activeLine": { backgroundColor: "rgba(10, 132, 255, 0.08)" },
    ".cm-gutters": {
      backgroundColor: "#18181a",
      color: "#525256",
      border: "none",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "rgba(10, 132, 255, 0.08)",
      color: "#9a9aa0",
    },
    ".cm-scroller": { fontFamily: "inherit" },
  },
  { dark: true },
);
