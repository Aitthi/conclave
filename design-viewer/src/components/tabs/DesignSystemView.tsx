import { useState } from "react";
import { Moon, Palette, Sun } from "lucide-react";
import type { Prototype } from "../../lib/types";
import { MONO, useTheme, type DarkTokens } from "../../lib/theme";
import { darkVars, tokensFromCss } from "../../lib/prototype";

function SectionHead({ title, count, c }: { title: string; count?: number; c: DarkTokens }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
      <span style={{ fontFamily: MONO, fontSize: 11, fontWeight: 600, letterSpacing: 0.5, textTransform: "uppercase", color: c.faint }}>
        {title}
      </span>
      {count != null && <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{count}</span>}
    </div>
  );
}

export function DesignSystemView({ prototype, projectId }: { prototype: Prototype; projectId: string }) {
  const { c } = useTheme();
  const [dark, setDark] = useState(false);
  // tokensFromCss/darkVars parse the raw `.arta/proto/theme.css` contents (Task 6's
  // `prototype.themeCss`) — the ONLY token source now. The old structured
  // `Prototype.tokens`/`Prototype.designSystem` fields were deleted in Task 13 along
  // with the rest of the HTML-string pipeline, so there is nothing left to merge a
  // fallback against.
  const t = tokensFromCss(prototype.themeCss);
  // The prototype's dark-theme token overrides + whether it supports a dark theme at all
  // (a `.dark{}` block in theme.css). When it does, offer a light/dark toggle that
  // re-renders the swatches in the chosen theme — so the dark side of the token system is
  // inspectable here. Component fragment CONTENT is no longer available client-side (the
  // assembled state only carries `componentNames`, not source), so there's nothing left to
  // scan for Tailwind `dark:` utility usage — that half of the old heuristic is dropped.
  const dv = darkVars(prototype.themeCss);
  const hasDark = Object.keys(dv).length > 0 || /\.dark\b/.test(prototype.themeCss || "");
  const showDark = hasDark && dark;

  const colors = t.colors || [];
  const typography = t.typography || [];
  const spacing = t.spacing || [];
  const radii = t.radii || [];
  const shadows = t.shadows || [];
  const fonts = t.fonts || [];
  // Component fragments (HTML strings, mustache {{>includes}}) are gone — a component is
  // now a real .tsx file under .arta/proto/components/, rendered live through the Task 4
  // Shell's own router. Only the name is available client-side; the gallery below renders
  // one small iframe per name at the shell's `#/_component/<name>` route.
  const componentNames = prototype.componentNames || [];

  const noTokens =
    !colors.length && !typography.length && !spacing.length && !radii.length && !shadows.length && !fonts.length && !componentNames.length;
  // A theme.css that has no parseable root/@theme vars (e.g. only class rules) still has
  // *something* to show — the stylesheet itself. Only truly-nothing is empty.
  const css = (prototype.themeCss || "").trim();
  const empty = noTokens && !css;

  if (empty) {
    return (
      <div className="flex h-full w-full items-center justify-center">
        <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
          <Palette size={28} />
          <div className="max-w-[320px] text-[13px] leading-relaxed">
            No design system yet — colours, type, spacing and components appear here once{" "}
            <span style={{ fontFamily: MONO }}>.arta/proto/theme.css</span> defines tokens (and{" "}
            <span style={{ fontFamily: MONO }}>.arta/proto/components/</span> has shared component files).
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto" style={{ background: c.bg }}>
      <div style={{ maxWidth: 1080, margin: "0 auto", padding: "26px 24px", display: "flex", flexDirection: "column", gap: 34 }}>
        {hasDark && (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", gap: 10, marginBottom: -18 }}>
            <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>Theme</span>
            <div style={{ display: "inline-flex", border: `1px solid ${c.border}`, borderRadius: 8, overflow: "hidden", background: c.panel }}>
              <button
                onClick={() => setDark(false)}
                title="Light"
                style={{ display: "grid", placeItems: "center", width: 36, height: 28, border: "none", cursor: "pointer", background: !dark ? c.accent : "transparent", color: !dark ? "#fff" : c.faint }}
              >
                <Sun size={14} />
              </button>
              <button
                onClick={() => setDark(true)}
                title="Dark"
                style={{ display: "grid", placeItems: "center", width: 36, height: 28, border: "none", cursor: "pointer", background: dark ? c.accent : "transparent", color: dark ? "#fff" : c.faint }}
              >
                <Moon size={14} />
              </button>
            </div>
          </div>
        )}
        {colors.length > 0 && (
          <section>
            <SectionHead title="Colors" count={colors.length} c={c} />
            <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill,minmax(150px,1fr))", gap: 12 }}>
              {colors.map((col) => {
                const dvVal = dv["color-" + col.name];
                const val = showDark && dvVal ? dvVal : col.value;
                return (
                  <div key={col.name} style={{ border: `1px solid ${c.border}`, borderRadius: 10, overflow: "hidden", background: c.panel }}>
                    <div style={{ height: 60, background: val, borderBottom: `1px solid ${c.border}` }} />
                    <div style={{ padding: "8px 10px" }}>
                      <div style={{ fontFamily: MONO, fontSize: 12, color: c.text }}>{col.name}</div>
                      <div style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>
                        {val}
                        {showDark && dvVal && <span style={{ color: c.accent }}> · dark</span>}
                      </div>
                      {col.description && <div style={{ fontSize: 11, color: c.dim, marginTop: 3, lineHeight: 1.45 }}>{col.description}</div>}
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {typography.length > 0 && (
          <section>
            <SectionHead title="Typography" count={typography.length} c={c} />
            <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
              {typography.map((ty) => (
                <div key={ty.name} style={{ borderBottom: `1px solid ${c.borderSoft}`, paddingBottom: 16 }}>
                  <div
                    style={{
                      color: c.text,
                      fontFamily: ty.family || undefined,
                      fontSize: ty.size || undefined,
                      fontWeight: ty.weight as number | undefined,
                      lineHeight: ty.lineHeight || undefined,
                      letterSpacing: ty.letterSpacing || undefined,
                    }}
                  >
                    {ty.sample || "The quick brown fox jumps over the lazy dog"}
                  </div>
                  <div style={{ fontFamily: MONO, fontSize: 11, color: c.faint, marginTop: 7 }}>
                    {ty.name}
                    {ty.size ? ` · ${ty.size}` : ""}
                    {ty.weight ? ` · ${ty.weight}` : ""}
                    {ty.lineHeight ? ` · lh ${ty.lineHeight}` : ""}
                    {ty.family ? ` · ${ty.family}` : ""}
                  </div>
                </div>
              ))}
            </div>
          </section>
        )}

        {fonts.length > 0 && (
          <section>
            <SectionHead title="Fonts" count={fonts.length} c={c} />
            <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
              {fonts.map((f) => (
                <div key={f.name}>
                  <div style={{ fontFamily: f.value, fontSize: 22, color: c.text }}>Aa — {f.name}</div>
                  <div style={{ fontFamily: MONO, fontSize: 11, color: c.faint, marginTop: 4 }}>{f.value}</div>
                </div>
              ))}
            </div>
          </section>
        )}

        {spacing.length > 0 && (
          <section>
            <SectionHead title="Spacing" count={spacing.length} c={c} />
            <div style={{ display: "flex", flexDirection: "column", gap: 9 }}>
              {spacing.map((s) => (
                <div key={s.name} style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <div style={{ width: s.value, height: 14, background: c.accent, borderRadius: 3, flexShrink: 0, minWidth: 2 }} />
                  <span style={{ fontFamily: MONO, fontSize: 12, color: c.text }}>{s.name}</span>
                  <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{s.value}</span>
                </div>
              ))}
            </div>
          </section>
        )}

        {radii.length > 0 && (
          <section>
            <SectionHead title="Radius" count={radii.length} c={c} />
            <div style={{ display: "flex", flexWrap: "wrap", gap: 16 }}>
              {radii.map((r) => (
                <div key={r.name} style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-start" }}>
                  <div style={{ width: 64, height: 64, background: c.panel2, border: `1px solid ${c.borderStrong}`, borderRadius: r.value }} />
                  <span style={{ fontFamily: MONO, fontSize: 11.5, color: c.text }}>{r.name}</span>
                  <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{r.value}</span>
                </div>
              ))}
            </div>
          </section>
        )}

        {shadows.length > 0 && (
          <section>
            <SectionHead title="Shadow" count={shadows.length} c={c} />
            <div style={{ display: "flex", flexWrap: "wrap", gap: 22 }}>
              {shadows.map((s) => (
                <div key={s.name} style={{ display: "flex", flexDirection: "column", gap: 8, alignItems: "flex-start" }}>
                  <div style={{ width: 96, height: 60, background: "#ffffff", border: "1px solid rgba(0,0,0,.06)", borderRadius: 10, boxShadow: s.value }} />
                  <span style={{ fontFamily: MONO, fontSize: 11.5, color: c.text }}>{s.name}</span>
                </div>
              ))}
            </div>
          </section>
        )}

        {componentNames.length > 0 && (
          <section>
            <SectionHead title="Components" count={componentNames.length} c={c} />
            {/* Live components — each name is a real .arta/proto/components/<name>.tsx file,
                rendered through the same Task 4 Shell every screen runs through, at its
                `#/_component/<name>` route (ComponentHost centers it in the frame). This is
                NOT affected by the light/dark toggle above: the toggle only recolors the
                token swatches/section (which read `dv` directly); the Shell boots its OWN
                theme from localStorage/prefers-color-scheme with no external override hook
                today, so every card here always renders in whatever theme the Shell itself
                picked — a disclosed limitation, not a silent gap. */}
            <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill,minmax(300px,1fr))", gap: 14 }}>
              {componentNames.map((name) => (
                <div key={name} style={{ border: `1px solid ${c.border}`, borderRadius: 12, overflow: "hidden", background: c.panel }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "9px 12px", borderBottom: `1px solid ${c.border}` }}>
                    <span style={{ fontFamily: MONO, fontSize: 12, fontWeight: 600, color: c.text, flex: 1 }}>{name}</span>
                  </div>
                  <iframe
                    title={name}
                    src={`/proto/index.html?project=${encodeURIComponent(projectId)}#/_component/${encodeURIComponent(name)}`}
                    sandbox="allow-scripts allow-forms allow-popups allow-same-origin"
                    style={{ width: "100%", height: 220, border: "none", background: "#fff" }}
                  />
                </div>
              ))}
            </div>
          </section>
        )}

        {noTokens && css && (
          <section>
            <SectionHead title="Stylesheet" c={c} />
            <p style={{ fontSize: 12, color: c.dim, marginBottom: 12, lineHeight: 1.5, maxWidth: 560 }}>
              This design system is authored as CSS without tokens the tab can parse — define
              them in <span style={{ fontFamily: MONO }}>.arta/proto/theme.css</span> under an{" "}
              <span style={{ fontFamily: MONO }}>@theme</span> (or{" "}
              <span style={{ fontFamily: MONO }}>:root</span>) block to see colours, type and
              spacing rendered here.
            </p>
            <pre
              style={{
                margin: 0,
                padding: "14px 16px",
                background: c.panel,
                border: `1px solid ${c.border}`,
                borderRadius: 10,
                overflowX: "auto",
                fontFamily: MONO,
                fontSize: 12,
                lineHeight: 1.6,
                color: c.text,
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
              }}
            >
              {css}
            </pre>
          </section>
        )}
      </div>
    </div>
  );
}
