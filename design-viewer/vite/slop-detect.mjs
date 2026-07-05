// ── Arta slop detector ────────────────────────────────────────────────────
// A self-contained, dependency-free anti-AI-slop detector. Given one screen's
// assembled HTML doc (tokens CSS + design sheet + body), it returns the design
// tells the AI most reliably emits — gradient text, side-stripe borders, stripe
// backgrounds, cramped tracking, nested cards, transition-all, uniform hover
// scale, emoji-as-icon, italic headings, placeholder names, mixed icon libs.
//
// Ported from the DETERMINISTIC subset of Hallmark's 58-gate slop test
// (MIT — github.com/Nutlope/hallmark) and aligned to impeccable's serious
// antipattern vocabulary, so it runs offline in the live loop (arta_design_review)
// AND in the eval grader (A5) with no `npx` / network / install dependency.
//
// Two callers rely on the finding shape { antipattern, severity, file, line,
// snippet, message }:
//   • mcp/server.mjs  arta_design_review — the craft eye in the live loop
//   • evals/grade.mjs A5_design — the deterministic CI floor (replaces the
//     external impeccable detect; `serious` is decided by id ∈ briefs.json's
//     serious_antipatterns, NOT by our severity — so we only emit a serious id
//     when the detection is precise enough to never fire on a clean design).

// ── helpers ─────────────────────────────────────────────────────────────────
function lineAt(doc, idx) {
  let n = 1;
  for (let i = 0; i < idx && i < doc.length; i++) if (doc[i] === "\n") n++;
  return n;
}
function snippetAt(doc, idx, span = 90) {
  const start = Math.max(0, idx - 10);
  return doc.slice(start, idx + span).replace(/\s+/g, " ").trim().slice(0, 120);
}
// Pull every class / className attribute value with its index in the doc.
function classAttrs(doc) {
  const out = [];
  const re = /\bclass(?:Name)?\s*=\s*"([^"]*)"|\bclass(?:Name)?\s*=\s*'([^']*)'/g;
  let m;
  while ((m = re.exec(doc))) out.push({ value: m[1] ?? m[2] ?? "", index: m.index });
  return out;
}

// ── gates ─────────────────────────────────────────────────────────────────
// `serious: true` gates emit an id that lives in briefs.json's serious set — they
// GATE the eval, so each must be precise (verified to never fire on the clean
// shipped demo). Everything else enriches the live review without gating.
//
// Two kinds: `re` (scan the whole doc) and `find` (custom logic, e.g. element
// co-occurrence or DOM nesting). Each pushes { id, severity, line, snippet, message }.

// gradient text: bg-clip:text (or Tailwind bg-clip-text) on a TRANSPARENT fill.
// A single solid colour is the rule; a clipped gradient headline is a top tell.
function findGradientText(doc, push) {
  // CSS form: background-clip:text near a transparent text fill.
  const reCss = /(?:-webkit-)?background-clip\s*:\s*text/gi;
  let m;
  while ((m = reCss.exec(doc))) {
    const around = doc.slice(Math.max(0, m.index - 200), m.index + 200);
    if (/(?:color|-webkit-text-fill-color)\s*:\s*transparent/i.test(around))
      push("gradient-text", "error", m.index, "background-clip:text + transparent fill");
  }
  // Tailwind form: bg-clip-text co-occurring with text-transparent on one element.
  for (const c of classAttrs(doc)) {
    if (/\bbg-clip-text\b/.test(c.value) && /\btext-transparent\b/.test(c.value))
      push("gradient-text", "error", c.index, "bg-clip-text + text-transparent (gradient headline)");
  }
}

// thick coloured side-stripe border on cards / callouts / list items — never
// intentional in good design (use a full border, a bg tint, or a leading marker).
function findSideStripe(doc, push) {
  let m;
  const reTw = /\bborder-[lr]-(?:4|8)\b/g;
  while ((m = reTw.exec(doc))) push("side-tab", "error", m.index, m[0] + " (side-stripe border)");
  const reCss = /border-(?:left|right)\s*:\s*(?:[4-9]|\d\d)px\s+solid/gi;
  while ((m = reCss.exec(doc))) push("side-tab", "error", m.index, snippetAt(doc, m.index, 40));
}

// stripe backgrounds — pure decoration, a reliable codex tell.
function findStripes(doc, push) {
  let m;
  const re = /repeating-(?:linear|radial)-gradient/gi;
  while ((m = re.exec(doc))) push("repeating-stripes-gradient", "error", m.index, snippetAt(doc, m.index, 50));
}

// extreme negative letter-spacing — letters touch, reads as cramped. Floor is
// -0.04em; flag ≤ -0.05em (CSS) and Tailwind tracking-tighter (-0.05em).
function findTracking(doc, push) {
  let m;
  const reCss = /letter-spacing\s*:\s*-(?:0?\.0[5-9]\d*|0?\.[1-9]\d*|[1-9][\d.]*)\s*em/gi;
  while ((m = reCss.exec(doc))) push("extreme-negative-tracking", "error", m.index, snippetAt(doc, m.index, 40));
  const reTw = /\btracking-tighter\b/g;
  while ((m = reTw.exec(doc))) push("extreme-negative-tracking", "error", m.index, "tracking-tighter (≤ -0.05em)");
}

// a card nested directly inside another card. A "card" = class names that read as
// a card (rounded + shadow/border, or an explicit `card`/`*-card` token). Nested
// cards are always a layout smell. Conservative: BOTH levels need the strong
// rounded+elevation signature, so a plain wrapper div doesn't trip it.
function isCardClass(v) {
  if (/\bcard\b|[-_]card\b/.test(v)) return true;
  const rounded = /\brounded-(?:lg|xl|2xl|3xl|\[)/.test(v) || /\brounded\b/.test(v);
  const elevated = /\bshadow(?:-(?:sm|md|lg|xl|2xl))?\b/.test(v) || /\bborder\b/.test(v);
  return rounded && elevated;
}
function findNestedCards(doc, push) {
  // Lightweight tag walk over container elements, tracking a stack of "is card".
  const re = /<\/?(?:div|section|article|li|a|aside)\b[^>]*>/gi;
  const stack = [];
  let m;
  while ((m = re.exec(doc))) {
    const tag = m[0];
    if (tag[1] === "/") { stack.pop(); continue; }
    const selfClose = /\/>\s*$/.test(tag);
    const cls = (tag.match(/\bclass(?:Name)?\s*=\s*"([^"]*)"|\bclass(?:Name)?\s*=\s*'([^']*)'/) || [])[1] || "";
    const card = isCardClass(cls);
    if (card && stack.some((s) => s.card))
      push("nested-cards", "error", m.index, "card inside card: " + snippetAt(doc, m.index, 50));
    if (!selfClose) stack.push({ card });
  }
}

// thin 1px border PLUS a wide soft drop shadow on the same element — the
// "ghost-card" tell. Precise: both declared in one inline style.
function findThinBorderWideShadow(doc, push) {
  const re = /style\s*=\s*"([^"]*)"/gi;
  let m;
  while ((m = re.exec(doc))) {
    const s = m[1];
    if (/border\s*:\s*1px\s+solid/i.test(s) && /box-shadow\s*:[^;"]*\b(?:1[6-9]|[2-9]\d)px/i.test(s))
      push("gpt-thin-border-wide-shadow", "error", m.index, "1px border + wide soft shadow (ghost card)");
  }
}

// ── warn / info gates (enrich the live review; NEVER gate the eval — ids are
// deliberately OUTSIDE briefs.json's serious set) ───────────────────────────
const AI_EMOJI = /[✨\u{1F680}⚡\u{1F525}\u{1F3AF}✅\u{1F389}\u{1F4A1}\u{1F44B}\u{1F4AA}\u{1F680}]/u;

const REGEX_GATES = [
  // transition: all / Tailwind transition-all — animates unknown properties, jank-prone.
  { id: "transition-all", severity: "warn", re: /transition\s*:\s*all|\btransition-all\b/gi, message: "transition:all — name the properties you animate" },
  // bouncy / overshoot easing on UI state — reserve overshoot for physical motion.
  { id: "overshoot-easing", severity: "warn", re: /cubic-bezier\(\s*[\d.]+\s*,\s*1\.[5-9]/gi, message: "overshoot easing on a UI transition" },
  // placeholder names / startup clichés.
  { id: "placeholder-name", severity: "warn", re: /\bJane Doe\b|\bJohn Smith\b|\bAcme\b|\bLorem ipsum\b/gi, message: "placeholder name / cliché — use real or plausible copy" },
  // dead image host: picsum.photos + source.unsplash.com were retired → every URL times out into a blank/skeleton tile.
  { id: "dead-image-host", severity: "warn", re: /picsum\.photos|source\.unsplash\.com/gi, message: "dead image host (picsum.photos / source.unsplash.com are retired) → blank; use images.unsplash.com/photo-<id> or loremflickr.com/<W>/<H>/<keyword>" },
  // brand / social icons were dropped from lucide core → render blank (a row of empty footer circles).
  { id: "brand-lucide-icon", severity: "warn", re: /data-lucide\s*=\s*["'](?:facebook|instagram|twitter|x|linkedin|youtube|github|gitlab|discord|slack|tiktok|dribbble|figma|twitch|whatsapp|telegram|pinterest|snapchat|reddit|medium|behance|threads)["']/gi, message: "brand icon dropped from lucide core → renders blank; use Iconify: <iconify-icon icon=\"simple-icons:<name>\">" },
  // a design-systems.md kit's literal accent shipped UNCHANGED — the kit hex is an example,
  // not a default. Left as-is, every build on that kit shares one palette (the convergence
  // tell). Ink #b3321a · Graphite #6e7bf2 · Clay #c0522e · Mist #0e8f86 · Signal #d8fe3e.
  { id: "unmodified-kit-default", severity: "warn", re: /#(?:b3321a|6e7bf2|c0522e|0e8f86|d8fe3e)\b/gi, message: "kit default accent used unchanged — give the project its OWN accent hue so two builds don't share a palette (the kit hex is an example, not the default)" },
  // marketing buzzword copy — generic filler that says nothing the product literally does.
  { id: "marketing-buzzword", severity: "warn", re: /\b(?:streamline your|empower your|supercharge your|unleash (?:your|the power)|leverage the power|built for the modern|trusted by (?:leading|the world)|best-in-class|industry-leading|world-class|enterprise-grade|next-generation|cutting-edge|revolutioniz\w+|game-chang\w+|mission-critical|future-proof|seamless(?:ly)? (?:experience|integrat\w+)|unlock your potential)\b/gi, message: "marketing buzzword — say what the product literally does, with a specific verb + noun" },
];

function findUniformHoverScale(doc, push) {
  const hits = [];
  const re = /\bhover:scale-1(?:0[5-9]|1\d|2\d)\b/g;
  let m;
  while ((m = re.exec(doc))) hits.push(m.index);
  if (hits.length >= 2) for (const i of hits) push("uniform-hover-scale", "warn", i, "uniform hover-scale across elements");
}
function findEmojiIcon(doc, push) {
  // emoji used where an icon belongs: inside a button/li/heading, or as a bullet.
  const re = new RegExp(AI_EMOJI.source, "gu");
  let m;
  while ((m = re.exec(doc))) push("emoji-icon", "warn", m.index, "emoji as icon — use a lucide glyph (" + doc.slice(m.index, m.index + 2).trim() + ")");
}
function findItalicHeading(doc, push) {
  const re = /<h[1-6]\b[^>]*\bclass(?:Name)?\s*=\s*["'][^"']*\bitalic\b/gi;
  let m;
  while ((m = re.exec(doc))) push("italic-heading", "warn", m.index, "italic heading — headers are roman; emphasise with weight/colour");
  const re2 = /<h[1-6]\b[^>]*style\s*=\s*"[^"]*font-style\s*:\s*italic/gi;
  while ((m = re2.exec(doc))) push("italic-heading", "warn", m.index, "italic heading (inline font-style:italic)");
}
function findMixedIconLibs(doc, push) {
  const libs = [];
  if (/\bdata-lucide\b/.test(doc)) libs.push("lucide");
  if (/\bclass(?:Name)?="[^"]*\bfa[srlbd]?-/.test(doc) || /\bfa-(?:solid|regular|brands)\b/.test(doc)) libs.push("fontawesome");
  if (/material-icons|material-symbols/.test(doc)) libs.push("material");
  if (/\bheroicon|hero-icon/.test(doc)) libs.push("heroicons");
  if (libs.length >= 2) push("mixed-icon-libs", "warn", doc.search(/\bdata-lucide\b|material-icons|fa-(?:solid|regular|brands)/), "two icon libraries on one screen: " + libs.join(" + "));
}
function findOverRounded(doc, push) {
  // cards / inputs / sections rounded past ~16px read as over-rounded (cap 12–16px).
  let m;
  const re = /\brounded-(?:\[(?:[3-9]\d|[1-9]\d\d)px\]|\[(?:[2-9](?:\.\d+)?rem)\])/g;
  while ((m = re.exec(doc))) push("over-rounded", "info", m.index, m[0] + " — cards top out ~16px");
  // Tailwind rounded-3xl = 24px; flag when paired with a border/shadow (i.e. a card).
  const reTw = /\brounded-3xl\b/g;
  while ((m = reTw.exec(doc))) {
    const around = doc.slice(Math.max(0, m.index - 120), m.index + 120);
    if (/\b(?:shadow|border)\b/.test(around)) push("over-rounded", "info", m.index, "rounded-3xl card (24px) — cards top out ~16px");
  }
}

// ── convergence gates — the "two different briefs come back identical" tells ──
// These don't fight a single bad element; they catch a design that converged on the
// saturated AI defaults instead of committing to its own identity. All `warn` (a nudge,
// never an eval gate): the fix is "make it yours", which is judgment, not a hard error.

// cream / warm off-white as the page background — the single most saturated AI-default
// neutral. A true off-white (chroma ~0) or a committed brand tint is fine; the warm beige
// that every brief drifts toward is the tell. Heuristic mirrors impeccable's isCream: light,
// warm-ordered (r≥g≥b), and *tinted* (6 ≤ r−b ≤ 48) — so pure white (#fff) and the kits'
// barely-warm off-whites (#fbfaf8 r−b=3, #fffefb r−b=4) DON'T trip it.
function isCreamHex(hex) {
  const h = hex.replace("#", "");
  if (h.length !== 6) return false;
  const r = parseInt(h.slice(0, 2), 16), g = parseInt(h.slice(2, 4), 16), b = parseInt(h.slice(4, 6), 16);
  return Math.min(r, g, b) >= 209 && r >= g && g >= b && r - b >= 6 && r - b <= 48;
}
function findCreamPalette(doc, push) {
  let m;
  const reCss = /(?:--color-bg|--bg|background(?:-color)?)\s*:\s*(#[0-9a-fA-F]{6})\b/g;
  while ((m = reCss.exec(doc))) if (isCreamHex(m[1])) push("cream-palette", "warn", m.index, "warm off-white / cream page background (" + m[1] + ") — the AI-default neutral; choose a true off-white or a committed brand surface");
  const reArb = /\bbg-\[(#[0-9a-fA-F]{6})\]/g;
  while ((m = reArb.exec(doc))) if (isCreamHex(m[1])) push("cream-palette", "warn", m.index, "cream bg-[" + m[1] + "] — the AI-default warm beige; pick a deliberate surface");
  const reTw = /\bbg-(?:amber|orange|yellow)-(?:50|100)\b/g;
  while ((m = reTw.exec(doc))) push("cream-palette", "warn", m.index, m[0] + " — warm off-white default; choose a deliberate surface, not the safe beige");
}

// generic AI purple/violet palette + indigo-500 — the loudest colour tell. Catches the hex
// set AND a Tailwind purple/violet gradient. (The kits' indigo #6e7bf2 is NOT in the set.)
function findAiPalette(doc, push) {
  let m;
  const reHex = /#(?:7c3aed|8b5cf6|a855f7|9333ea|7e22ce|6d28d9|6366f1|818cf8|764ba2|667eea)\b/gi;
  while ((m = reHex.exec(doc))) push("ai-color-palette", "warn", m.index, "generic AI purple / indigo hex (" + doc.substr(m.index, 7) + ") — choose a distinctive brand colour");
  for (const c of classAttrs(doc)) {
    if (/\bbg-gradient-to-/.test(c.value) && /\b(?:from|via|to)-(?:purple|violet|fuchsia)-/.test(c.value))
      push("ai-color-palette", "warn", c.index, "purple/violet gradient (" + (c.value.match(/\b(?:from|via|to)-(?:purple|violet|fuchsia)-\d+/) || [""])[0] + ") — the most recognizable AI palette");
  }
}

// em-dash overuse — the LLM-prose punctuation tell. Counts only the real em-dash glyph
// (U+2014), never `--` (so CSS custom props like `--color-bg` don't trip it).
function findEmDashOveruse(doc, push) {
  const idxs = [];
  let i = -1;
  while ((i = doc.indexOf("—", i + 1)) !== -1) idxs.push(i);
  if (idxs.length >= 5) push("em-dash-overuse", "warn", idxs[0], "em-dash overuse (" + idxs.length + ") — vary with commas, colons, periods, parentheses");
}

// ── formerly-phantom tells — declared in briefs.json's serious set but with NO detector,
// so they never fired. Now implemented. Only `hero-eyebrow-chip` stays serious (it's
// unambiguous: a clean design never needs a pulsing status eyebrow). The rest are `warn`
// (real tells, but a good design CAN use a 3-up feature grid, an oversized display hero,
// a featured-tier accent border, or muted-on-dark text) — so they enrich the live review
// without gating the eval, and were removed from briefs.json serious_antipatterns to match.

// hero eyebrow chip — a pulsing ping-dot status pill, or a rounded-full dot+micro-label
// chip sitting right above the hero <h1>. The canonical AI landing eyebrow ("● NOW IN BETA").
function findHeroEyebrowChip(doc, push) {
  let m;
  const rePing = /\banimate-ping\b/g;
  while ((m = rePing.exec(doc))) push("hero-eyebrow-chip", "error", m.index, "pulsing ping-dot status chip — the AI 'live/beta/new' eyebrow; state the status in plain words or drop it");
  const reChip = /<[^>]*\brounded-full\b[^>]*>[\s\S]{0,240}?<h1\b/gi;
  while ((m = reChip.exec(doc))) {
    if (/\b[hw]-(?:1|1\.5|2)\b[^>]*rounded-full|[•●]/.test(m[0]))
      push("hero-eyebrow-chip", "error", m.index, "eyebrow chip (dot + micro-label pill) above the hero — lead with the headline, not a badge");
  }
}

// repeated section kickers — a tiny uppercase, wide-tracked micro-label opening section
// after section ("WHY TEAMS SWITCH" / "THE PROCESS"). One is fine; ≥3 is the templated tell.
function findRepeatedKickers(doc, push) {
  const idxs = [];
  for (const c of classAttrs(doc)) {
    const v = c.value;
    if (/\buppercase\b/.test(v) && (/\btracking-(?:wider|widest)\b/.test(v) || /\btracking-\[0?\.[12]\d*em\]/.test(v)) && /\btext-(?:\[1[01]px\]|xs|\[0\.\d+rem\])\b/.test(v))
      idxs.push(c.index);
  }
  if (idxs.length >= 3) for (const i of idxs) push("repeated-section-kickers", "warn", i, "repeated uppercase tracked section kicker (" + idxs.length + "×) — the 'eyebrow on every section' template; let the headings open sections");
}

// icon-tile-stack — ≥3 feature blocks each led by an icon inside a small rounded SQUARE
// tile (not a circle). The AI "three features in a row" fingerprint.
function findIconTileStack(doc, push) {
  const tiles = [];
  for (const c of classAttrs(doc)) {
    const v = c.value;
    if (/\b(?:h-|w-|size-)(?:8|9|10|11|12|14|16)\b/.test(v) && /\brounded-(?:md|lg|xl|2xl)\b/.test(v) && !/\brounded-full\b/.test(v) && /\bitems-center\b/.test(v) && /\bjustify-center\b/.test(v))
      tiles.push(c.index);
  }
  if (tiles.length >= 3) for (const i of tiles) push("icon-tile-stack", "warn", i, "icon-in-rounded-tile stack (" + tiles.length + "×) — the AI feature-grid fingerprint; vary the blocks or drop the tile chrome");
}

// oversized display h1 — text-8xl/9xl or an inline font-size ≥ ~6rem on the hero. WARN only:
// a bold-display brand uses oversized type on purpose, so this flags it to re-check mobile.
function findOversizedH1(doc, push) {
  let m;
  const reTw = /<h1\b[^>]*\b(text-(?:8xl|9xl))\b/gi;
  while ((m = reTw.exec(doc))) push("oversized-h1", "warn", m.index, m[1] + " hero — verify it doesn't overflow on a phone; oversized type is fine if it's the brand");
  const reH1 = /<h1\b[^>]*style\s*=\s*"([^"]*)"/gi;
  while ((m = reH1.exec(doc))) {
    if ([...m[1].matchAll(/(\d+(?:\.\d+)?)rem/g)].some((x) => +x[1] >= 6)) push("oversized-h1", "warn", m.index, "hero font-size ≥6rem — verify mobile wrapping; oversized is fine if intentional");
  }
}

// border-accent-on-rounded — a brand-COLOURED full border on a rounded card. One featured
// tier is legit, so WARN (not gate): ≥2 such cards is the "accent border on everything" tell.
function findBorderAccentRounded(doc, push) {
  const hits = [];
  for (const c of classAttrs(doc)) {
    const v = c.value;
    const rounded = /\brounded-(?:md|lg|xl|2xl|3xl)\b/.test(v);
    const hasBorder = /\bborder(?:-2|-4)?\b/.test(v);
    const accent = /\bborder-\[var\(--color-(?:accent|primary|brand)\)\]/.test(v) || /\bborder-(?:purple|violet|fuchsia|indigo|blue|sky|cyan|emerald|teal|green|lime|amber|orange|red|rose|pink)-(?:3|4|5|6)00\b/.test(v);
    const neutral = /\bborder-(?:gray|zinc|slate|neutral|stone|white|black)\b/.test(v);
    if (rounded && hasBorder && accent && !neutral) hits.push(c.index);
  }
  if (hits.length >= 2) for (const i of hits) push("border-accent-on-rounded", "warn", i, "accent-coloured border on a rounded card (" + hits.length + "×) — reserve the brand border for the selected/featured card; tint or hairline the rest");
}

// low-contrast — needs a full colour engine to gate; we flag the precise, unambiguous
// subset: a known light-gray text utility (-300/-400) NOT in a dark-surface context.
function findLowContrast(doc, push) {
  const re = /\btext-(?:gray|zinc|slate|neutral|stone)-(?:300|400)\b/g;
  let m;
  while ((m = re.exec(doc))) {
    const around = doc.slice(Math.max(0, m.index - 180), m.index + 60);
    if (!/\bbg-(?:black|(?:gray|zinc|slate|neutral|stone)-(?:8|9)00)\b|\bbg-\[#0[0-9a-f]|\bdark\b/.test(around))
      push("low-contrast", "warn", m.index, m[0] + " as text — below 4.5:1 on a light surface; use -600/-700 for body copy");
  }
}

// gray-on-color — muted gray text on a saturated/brand surface reads muddy. Precise
// co-occurrence in one class (gray text + a brand/saturated bg).
function findGrayOnColor(doc, push) {
  for (const c of classAttrs(doc)) {
    const v = c.value;
    if (/\btext-(?:gray|zinc|slate|neutral|stone)-(?:300|400|500)\b/.test(v) && (/\bbg-\[var\(--color-(?:accent|primary|brand)\)\]/.test(v) || /\bbg-(?:purple|violet|indigo|blue|sky|cyan|emerald|teal|green|lime|amber|orange|red|rose|pink)-(?:5|6|7)00\b/.test(v)))
      push("gray-on-color", "warn", c.index, "muted gray text on a brand-coloured surface — use white / near-white or a tint of the surface colour");
  }
}

// status-dot pill — a small pill/badge that LEADS with a tiny coloured status dot + a
// short label (● Live / ● v1.4.0 / ● all systems operational). The AI "liveness" chip:
// it manufactures a sense of real-time status on a static prototype. WARN — a real product
// site legitimately shows one (a status page, a build badge), so surface it for judgment,
// don't gate. The hero-above-the-fold variant is the stricter, gating `hero-eyebrow-chip`;
// this catches the same tell in chrome (topbar / footer) where that one never looks.
// Discrimination: the dot must be the pill's FIRST child — a bare status dot in a table
// row or a status list (dot NOT wrapped in a pill) is a legit indicator and stays silent.
function findStatusDotPill(doc, push) {
  // a pill-like container (custom `.pill`/`.badge`/`.chip` class, or a Tailwind rounded-full
  // chip) whose first child is a status dot (a `.dot` element, or a tiny rounded-full swatch
  // with a background colour).
  const re = /<(?:span|div|a|p)\b[^>]*\bclass="[^"]*(?:\b(?:pill|badge|chip)\b|\brounded-full\b)[^"]*"[^>]*>\s*(<(?:span|i|div)\b[^>]*>)/gi;
  let m;
  while ((m = re.exec(doc))) {
    const dot = m[1];
    const isDot =
      /\bclass="[^"]*\bdot\b[^"]*"/.test(dot) ||
      (/\brounded-full\b/.test(dot) && /\b(?:h|w|size)-(?:1|1\.5|2|2\.5)\b/.test(dot) && /\bbg-/.test(dot));
    if (isDot)
      push("status-dot-pill", "warn", m.index, "status-dot pill (● + micro-label badge) — the AI 'liveness' chip; say the status in plain words or drop the dot");
  }
}

const CUSTOM_GATES = [
  findGradientText, findSideStripe, findStripes, findTracking, findNestedCards,
  findThinBorderWideShadow, findUniformHoverScale, findEmojiIcon, findItalicHeading,
  findMixedIconLibs, findOverRounded, findCreamPalette, findAiPalette, findEmDashOveruse,
  findHeroEyebrowChip, findRepeatedKickers, findIconTileStack, findOversizedH1,
  findBorderAccentRounded, findLowContrast, findGrayOnColor, findStatusDotPill,
];

// Human-readable titles for ids the live review surfaces.
const TITLES = {
  "gradient-text": "Gradient text headline",
  "side-tab": "Coloured side-stripe border",
  "repeating-stripes-gradient": "Stripe-gradient background",
  "extreme-negative-tracking": "Cramped letter-spacing",
  "nested-cards": "Card nested inside a card",
  "gpt-thin-border-wide-shadow": "1px border + wide soft shadow",
  "transition-all": "transition: all",
  "overshoot-easing": "Overshoot easing on UI state",
  "placeholder-name": "Placeholder name / cliché",
  "dead-image-host": "Dead image host (picsum / source.unsplash)",
  "brand-lucide-icon": "Brand icon not in lucide core (blank)",
  "uniform-hover-scale": "Uniform hover-scale",
  "emoji-icon": "Emoji used as an icon",
  "italic-heading": "Italic heading",
  "mixed-icon-libs": "Mixed icon libraries",
  "over-rounded": "Over-rounded card",
  "cream-palette": "Cream / warm off-white default background",
  "ai-color-palette": "Generic AI purple / indigo palette",
  "em-dash-overuse": "Em-dash overuse",
  "unmodified-kit-default": "Kit default accent (make it the project's own)",
  "marketing-buzzword": "Marketing buzzword copy",
  "hero-eyebrow-chip": "Hero eyebrow chip (dot + micro-label)",
  "repeated-section-kickers": "Repeated uppercase section kickers",
  "icon-tile-stack": "Icon-in-tile feature grid",
  "oversized-h1": "Oversized display hero",
  "border-accent-on-rounded": "Accent border on a rounded card",
  "low-contrast": "Low-contrast text",
  "gray-on-color": "Gray text on a coloured surface",
  "status-dot-pill": "Status-dot 'liveness' pill",
};

// Run CUSTOM_GATES + REGEX_GATES over a text string, dedupe, return findings. The one
// shared loop both public entry points (`detectSlop`, `detectSlopJsx`) delegate to —
// they differ only in what preprocessing (if any) happens to `text` before it gets here.
function runGates(text, opts = {}) {
  const file = opts.file || "";
  const findings = [];
  const seen = new Set(); // dedupe identical id@line
  const push = (id, severity, index, message) => {
    const line = lineAt(text, index);
    const key = id + "@" + line + "@" + (message || "");
    if (seen.has(key)) return;
    seen.add(key);
    findings.push({ antipattern: id, severity, file, line, snippet: snippetAt(text, index), message: TITLES[id] ? TITLES[id] + " — " + message : message });
  };
  for (const g of CUSTOM_GATES) g(text, push);
  for (const g of REGEX_GATES) {
    let m;
    g.re.lastIndex = 0;
    while ((m = g.re.exec(text))) push(g.id, g.severity, m.index, g.message);
  }
  return findings;
}

/**
 * Detect anti-slop tells in one assembled screen doc.
 * @param {string} doc  the screen's HTML (ideally tokens CSS + sheet + body)
 * @param {{file?: string}} [opts]
 * @returns {Array<{antipattern:string,severity:string,file:string,line:number,snippet:string,message:string}>}
 */
export function detectSlop(doc, opts = {}) {
  const text = typeof doc === "string" ? doc : "";
  return runGates(text, opts);
}

// Replace every non-newline character in `s` with a space, keeping every `\n` — used to
// blank out a span (a JSX comment, a template literal) while preserving line positions so
// `lineAt()` still maps back to the real source line.
function blankKeepingNewlines(s) {
  return s.replace(/[^\n]/g, " ");
}

// ── JSX preprocessing for detectSlopJsx ─────────────────────────────────────
// Transforms JSX source into text the SAME gate loop (`runGates`) can scan, under the
// hard constraint that the output has the EXACT SAME LINE COUNT as the input — every
// substitution only ever replaces same-line (or newline-preserving multi-line) content,
// never adding/removing a `\n`, so a finding's `line` still points at the real JSX line.
function preprocessJsx(src) {
  let text = src;

  // className= → class= (a straight literal replace; doesn't touch newlines at all,
  // so it's always safe to do first).
  text = text.replace(/\bclassName(?=\s*=)/g, "class");

  // {/* ... */} JSX comments — blank the whole span (delimiters included) except for
  // any `\n` inside it, so a slop-shaped phrase inside a comment can never fire, while
  // a comment spanning N lines still spans N lines afterward.
  text = text.replace(/\{\s*\/\*[\s\S]*?\*\/\s*\}/g, (m) => blankKeepingNewlines(m));

  // Template-literal classNames: class={`literal ${expr} literal`} → class="literal  literal".
  // Concatenate the literal quasi parts, drop every ${...} expression outright (fidelity to
  // the exact mixed string isn't required — only that literal Tailwind-class-shaped text
  // inside the template reaches the gates). Any `\n` inside the original span (quasis OR
  // expressions) is preserved verbatim so the replacement spans the same number of lines.
  text = text.replace(/\bclass=\{(\s*)`([\s\S]*?)`(\s*)\}/g, (whole, leading, inner, trailing) => {
    // Pull the literal quasi segments (text between `${` `}` pairs) and the newlines that
    // lived inside dropped ${...} expressions, so line count is preserved either way.
    let literal = "";
    let i = 0;
    while (i < inner.length) {
      const exprStart = inner.indexOf("${", i);
      if (exprStart === -1) {
        literal += inner.slice(i);
        break;
      }
      literal += inner.slice(i, exprStart);
      // Find the matching close brace (naive but sufficient: no nested template literals
      // expected in a className expression).
      let depth = 1;
      let j = exprStart + 2;
      while (j < inner.length && depth > 0) {
        if (inner[j] === "{") depth++;
        else if (inner[j] === "}") depth--;
        j++;
      }
      // Keep only any newlines inside the dropped expression so total line count is stable.
      literal += inner.slice(exprStart, j).replace(/[^\n]/g, "");
      i = j;
    }
    const replacement = `class="${literal}"`;
    // `inner`'s newlines (quasi text plus the preserved expression newlines above) are
    // already all present in `replacement` by construction — this only tops up defensively
    // in case that ever isn't exact, so the span's total line count stays correct.
    const wantInnerNl = (inner.match(/\n/g) || []).length;
    const gotInnerNl = (replacement.match(/\n/g) || []).length;
    const middle = replacement + "\n".repeat(Math.max(0, wantInnerNl - gotInnerNl));
    // The `\s*` padding OUTSIDE the backticks — between `{` and the opening backtick, and
    // between the closing backtick and `}` — can itself contain newlines (e.g. `class={` on
    // its own line, literal starting on the next). Restore those newlines IN PLACE: leading
    // ones BEFORE the flattened replacement, trailing ones AFTER it — never lumped at the
    // tail — so content inside the span keeps mapping back to its true source line.
    const leadingNl = "\n".repeat((leading.match(/\n/g) || []).length);
    const trailingNl = "\n".repeat((trailing.match(/\n/g) || []).length);
    return leadingNl + middle + trailingNl;
  });

  // General class={<expr>} expressions — cn()/clsx()/twMerge() calls, ternaries, `&&`
  // arms — SKILL.md's own recommended pattern for conditional classNames, which the
  // plain-template-literal case above doesn't cover. Every quoted/backtick string
  // literal ANYWHERE inside the `{...}` span is pulled out and concatenated into a
  // flattened `class="..."`, e.g. `class={cn("rounded-full", live && "bg-emerald-50")}`
  // → `class="rounded-full bg-emerald-50"`. Brace matching is naive depth-counting (same
  // simplification as the template-literal case above: no nested template literals or
  // braces-inside-strings expected in a className expression) — sufficient for real
  // Tailwind class arguments. Runs a manual scan (not `.replace()`) because the
  // replacement must consume everything up to the matching `}`, not just the `class={`
  // token a regex match would give us.
  {
    // `\b` keeps this anchored to a real `class=` attribute, not merely a substring
    // (e.g. it won't fire in the middle of some other `...Class={` prop name).
    const markerRe = /\bclass\s*=\s*\{/g;
    let out = "";
    let i = 0;
    let mm;
    while ((mm = markerRe.exec(text))) {
      const idx = mm.index;
      out += text.slice(i, idx);
      const exprStart = idx + mm[0].length;
      let depth = 1;
      let j = exprStart;
      while (j < text.length && depth > 0) {
        if (text[j] === "{") depth++;
        else if (text[j] === "}") depth--;
        j++;
      }
      const inner = text.slice(exprStart, j - 1);
      const literals = [];
      const strRe = /"([^"]*)"|'([^']*)'|`([^`]*)`/g;
      let sm;
      while ((sm = strRe.exec(inner))) {
        const lit = sm[1] ?? sm[2] ?? sm[3] ?? "";
        // Drop any ${...} expression inside a backtick quasi (mirrors the plain
        // template-literal case above) — only literal Tailwind-class-shaped text matters.
        literals.push(lit.replace(/\$\{[^}]*\}/g, " "));
      }
      // Total newlines inside `inner` (wherever they sit — whitespace, string literals,
      // or dropped code) is exactly the span's original line count, so counting them in
      // bulk and appending after the flattened value keeps every LATER line's number
      // correct, same invariant the rest of this function upholds.
      const innerNl = (inner.match(/\n/g) || []).length;
      out += `class="${literals.join(" ")}"` + "\n".repeat(innerNl);
      i = j;
      markerRe.lastIndex = j; // resume scanning after the consumed expression, not inside it
    }
    out += text.slice(i);
    text = out;
  }

  return text;
}

/**
 * Detect anti-slop tells in one screen/component's JSX source. Reuses the SAME gate list
 * as `detectSlop` (via `runGates`) — this is only a different input adapter, not a fork.
 * @param {string} src  the raw .tsx source
 * @param {{file?: string}} [opts]
 * @returns {Array<{antipattern:string,severity:string,file:string,line:number,snippet:string,message:string}>}
 */
export function detectSlopJsx(src, opts = {}) {
  const text = typeof src === "string" ? preprocessJsx(src) : "";
  return runGates(text, opts);
}

// Convenience: detect across many { file, doc } screens at once.
export function detectScreens(screens) {
  const all = [];
  for (const s of screens) all.push(...detectSlop(s.doc, { file: s.file }));
  return all;
}

export default detectSlop;
