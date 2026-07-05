---
name: Design Craft
description: What makes an interface look designed, not AI-generated — the anti-slop vocabulary, token/type/colour/layout discipline, and the six-axis critique rubric. Load before authoring any design screen; run `conclave design review <ws>` before calling a design done.
mandatory: false
---

The gap between a screen that looks *designed* and one that looks like "an AI made
a webpage" is a finite, learnable set of tells plus a matter of taste. This skill is
both: the **detector vocabulary** (absolute tells `conclave design review` catches
deterministically) and the **critique rubric** (the taste the detector can't see).
Composes with Design Canvas (the `design/` contract) and Arta Designer (the role).
Run BOTH on every screen — detector as the floor, rubric as the ceiling.

## The anti-slop vocabulary — mirror of the review detector

`conclave design review <ws>` scans `design/screens/*.tsx` + `components/*.tsx` +
`theme.css` and reports findings `{ antipattern, severity, file, line, snippet,
message }`. The names below ARE the finding ids — so when the review reds one, you
already know the fix. Don't ship them; rework the element.

**Serious (the review gate fails on these — fix every one):**
- **gradient-text** — a `bg-clip-text` + `text-transparent` (or `background-clip:text`
  + transparent fill) gradient headline. Use ONE solid colour; emphasise with weight/size.
- **side-tab** — a thick coloured `border-l-4`/`border-r-8` side-stripe on a card, alert,
  or active row. Use a full border, a background tint, or a leading marker — never a side bar.
- **repeating-stripes-gradient** — a `repeating-linear/radial-gradient` decorative stripe bg.
- **extreme-negative-tracking** — `tracking-tighter` or `letter-spacing ≤ -0.05em`; letters
  touch. The floor is about -0.04em.
- **nested-cards** — a card (rounded + shadow/border) directly inside another card. Flatten:
  the inner content doesn't need its own elevation.
- **gpt-thin-border-wide-shadow** — a 1px border PLUS a wide soft drop-shadow on one element
  (the "ghost card"). Pick one: a hairline border OR a shadow, not both.
- **hero-eyebrow-chip** — a pulsing `animate-ping` dot, or a rounded-full dot + micro-label
  pill sitting right above the hero `<h1>` ("● NOW IN BETA"). Lead with the headline; state a
  real status in plain words inline.

**Warn (judge in context — a good design *can* use some of these once, on purpose):**
- **transition-all** — `transition: all`; name the properties you animate.
- **overshoot-easing** — bouncy `cubic-bezier(…,1.5+…)` on a UI state; reserve overshoot for motion.
- **uniform-hover-scale** — the same `hover:scale-105` on every card; vary or drop it.
- **emoji-icon** — an emoji where an icon belongs; use a `lucide-react` glyph.
- **italic-heading** — an italic `<h1>`–`<h6>`; headers are roman, emphasise with weight/colour.
- **over-rounded** — a card rounded past ~16px (`rounded-3xl`, big `rounded-[Npx]`); cards top
  out 12–16px (pills are fine).
- **mixed-icon-libs** — two icon libraries on one screen; stay in `lucide-react`.
- **cream-palette** — a warm off-white / beige page background (`bg-amber/orange/yellow-50`, a
  cream hex), the convergence default for "premium/editorial". Keep a true off-white or commit
  to a deep brand surface; carry warmth in the accent, type, imagery.
- **ai-color-palette** — the generic AI purple/indigo (a `from-purple-…via-violet-…` gradient or
  the tell-tale hexes). Choose a distinctive brand hue.
- **unmodified-kit-default** — a starter kit's *example* accent shipped unchanged, so every build
  shares one palette. Give the project its OWN accent hue.
- **em-dash-overuse** — 5+ em-dashes; vary with commas, colons, periods, parentheses.
- **marketing-buzzword** — "streamline your / best-in-class / seamless experience"; say what the
  product literally does, specific verb + noun.
- **placeholder-name** — "Jane Doe / Acme / Lorem ipsum"; use real, plausible copy.
- **dead-image-host** — `picsum.photos` / `source.unsplash.com` (retired → blank); use
  `images.unsplash.com/photo-<id>` or `loremflickr.com/<W>/<H>/<keyword>`.
- **brand-lucide-icon** — social/brand names aren't in lucide core → render blank.
- **repeated-section-kickers** — a tiny uppercase tracked eyebrow opening 3+ sections; let the
  headings open sections.
- **icon-tile-stack** — 3+ feature blocks each led by an icon in a small rounded square tile (the
  "three features in a row" fingerprint); vary the blocks.
- **oversized-h1** — `text-8xl/9xl` hero; fine if it's the brand, but verify mobile wrapping.
- **border-accent-on-rounded** — an accent border on 2+ rounded cards; reserve it for the
  selected/featured one, hairline the rest.
- **low-contrast** — `text-gray-300/400` body on a light surface (below 4.5:1); use -600/-700.
- **gray-on-color** — muted gray text on a brand surface; use white/near-white or a surface tint.
- **status-dot-pill** — a badge leading with a coloured "● Live" dot (manufactured liveness); say
  the status in words or drop the dot.

*(The tells above are ported from **Hallmark** — MIT, github.com/Nutlope/hallmark — and the
impeccable design vocabulary; `conclave design review` encodes the deterministic subset as
gates, so prose and detector agree.)*

## Token discipline — one source of truth

- **Pick a brand-grade design language FIRST.** Generic grays + a blue accent is the #1
  "AI made this" tell. Set real tokens in `design/theme.css`'s `@theme {}` — a distinctive
  accent, tinted neutrals, a type scale, radius, shadow — and build every screen from them.
- **No raw hex in a screen body.** No `style={{ color: "#1a1a1a" }}`, no `bg-[#f5f5f5]`. Every
  colour is a token-derived utility (`bg-accent`, `text-heading`) or `var(--color-*)`; need a
  shade the kit lacks, add it to `theme.css`. The repeated **semantic palette** (status/severity
  colours) lives in tokens too, defined once — inlining the same hex 10× across screens is the
  classic "has a design system but doesn't use it" miss.
- **Commit to ONE colour strategy on purpose:** *restrained* (tinted neutrals + one accent
  ≤10%), *committed* (one saturated colour on 30–60% of the surface), *full palette* (3–4 named
  roles), or *drenched* (the surface IS the colour). Product/tool UI floors at restrained;
  brand/marketing earns committed-or-louder. Beige + one accent on a bold brief is a hedge.
- **Make it *this* project, not its category (the reflex check).** *First-order:* could someone
  guess the palette + type from the category alone ("dashboard → dark + indigo")? If yes, rework
  it. *Second-order:* could they guess it from the category plus the obvious anti-move? Keep
  going until neither predicts it. Anchor to one concrete **scene sentence** — who uses this,
  where, in what light ("a night-shift dispatcher in a dim ops room", never "modern and clean")
  — and name 2–3 ways this build looks unlike the generic version of its category.

## Type, layout, motion — commit, don't hedge

- **Type scale must jump.** Display/hero ≥ ~2× the body and heavy; labels small, uppercase,
  muted, slightly spaced. When H1, card titles, metric values and body collapse into one narrow
  band, hierarchy dies and it reads templated. Use the extremes (a 48px+ hero over an 11px label).
- **A Latin display face needs a script fallback for non-Latin text** (Thai/CJK/Arabic): add a
  matching fallback in the font stack, or reserve the Latin face for genuinely-Latin runs.
- **One spacing rhythm; consistent radius/shadow/motion across screens.** Generous whitespace
  and one confident accent beat many timid ones.
- **Interactive state must read.** A selected/active item shifts the **fill** (a tint), not just
  a border; use a solid control (filled radio/check, not a ghost outline) — on dark themes a
  border-only "selected" is nearly invisible. Never mark it with a `border-left` bar: that is the
  banned **side-tab**, and an active nav row is where the reflex sneaks in.
- **Real content, never lorem/placeholder** — real copy, names, prices. **Images: a REAL image
  or an intentional token-tinted skeleton, never a bare solid fill** (a flat rectangle where a
  photo belongs reads unfinished).

## Vary the screen shape — structural, not colour

The #1 reason a multi-screen build reads "templated" is that every screen is the same shape.
**Name the shape before you build each screen** and make consecutive screens differ
structurally: *dashboard/bento* (tiles of varying size), *master–detail split*, *feed*,
*index/browse*, *detail/profile*, *workbench*, *stepped flow*, *focused/empty* (centered, no
dead band), *table/data-dense*, *marketing*. Same nav on every screen is fine (factor it into a
component); the same *body skeleton* twice is the failure — if you'd name the same shape twice
in a row, change one.

## The critique rubric — score before you look

Six axes, each **1–5**, scored BLIND (from what you meant to build) before you see the render.
**Anything < 3 triggers a revision pass.** A 3 is "fine", a 4 is "designed", a 5 is "couldn't
be anyone else's product." Aim for 4s.

- **Philosophy** — a stance you could state in one line, or just assembled defaults?
- **Hierarchy** — squint: primary/secondary/tertiary read in 2 seconds?
- **Execution** — detector-clean AND tight (one rhythm, all states, aligned)? A standing serious
  finding caps this at 2.
- **Specificity** — the reflex check: first-order NO, second-order NO? (The core axis — no
  detector equivalent.)
- **Restraint** — does every element earn its pixel? Delete every one whose removal doesn't
  make the screen worse. App screens usually need no footer, a slim status strip at most.
- **Variety** — shape matches the job and differs from its neighbours?

**Workflow:** score blind → read the render (hierarchy, contrast, spacing, overlap, dead band),
re-score any axis the pixels contradict → run `conclave design review <ws>` and fix every
serious finding → revise anything < 3 and re-check. Two passes is normal; a **third pass means
the DIRECTION is wrong** — rethink philosophy/specificity, don't grind pixels on a flawed premise.

The detector is the floor (nothing slips), the rubric is the ceiling (is it *designed*, and is
it *this* product). A detector-clean, rubric-flat screen is the exact failure this skill exists
to catch. **Run `conclave design review <ws>` and clear the rubric before you call a design done.**
