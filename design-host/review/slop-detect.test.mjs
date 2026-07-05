// design-host/review/slop-detect.test.mjs
// Vendored verbatim from arta's mcp/slop-detect.test.mjs (MIT, see slop-detect.mjs
// header) — the ONLY change is the test runner: arta ran this under `bun:test`,
// but Lane R's gate is `node design-host/review/slop-detect.test.mjs`, and node
// cannot load the `bun:test` builtin. So we use node's built-in `node:test` and a
// tiny local `expect` shim covering exactly the matchers this suite uses
// (`toBe`, `toBeDefined`, `toBeGreaterThan`). The assertion BODIES are unchanged.
import { describe, test } from "node:test";
import assert from "node:assert/strict";
import { detectSlop, detectSlopJsx } from "./slop-detect.mjs";

function expect(received) {
  return {
    toBe: (expected) => assert.strictEqual(received, expected),
    toBeDefined: () => assert.notStrictEqual(received, undefined),
    toBeGreaterThan: (n) =>
      assert.ok(received > n, `expected ${received} to be greater than ${n}`),
  };
}

describe("detectSlopJsx", () => {
  test("fires the same gradient-text gate as detectSlop, with the JSX line number", () => {
    const jsx = [
      "export const meta = { title: \"Cart\" };",
      "",
      "export default function Cart() {",
      "  return (",
      "    <h1 className=\"bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent\">",
      "      Your cart",
      "    </h1>",
      "  );",
      "}",
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "cart.tsx" });
    const hit = findings.find((f) => f.antipattern === "gradient-text");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(5);
    expect(hit.file).toBe("cart.tsx");

    // Same gate fires on the HTML equivalent via detectSlop.
    const html = `<h1 class="bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent">Your cart</h1>`;
    const htmlFindings = detectSlop(html, { file: "cart.html" });
    expect(htmlFindings.some((f) => f.antipattern === "gradient-text")).toBe(true);
  });

  test("does not trip on slop-shaped text inside a JSX comment", () => {
    const jsx = [
      "export default function Card() {",
      "  return (",
      "    <div>",
      "      {/* bg-clip-text text-transparent */}",
      "      <p>hello</p>",
      "    </div>",
      "  );",
      "}",
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "card.tsx" });
    expect(findings.some((f) => f.antipattern === "gradient-text")).toBe(false);
  });

  test("preserves line count / mapping across a multi-line JSX comment", () => {
    const jsx = [
      "export default function Card() {",
      "  return (",
      "    <div>",
      "      {/*",
      "        bg-clip-text text-transparent",
      "        multi-line comment",
      "      */}",
      "      <h1 className=\"bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent\">Hi</h1>",
      "    </div>",
      "  );",
      "}",
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "card.tsx" });
    const hit = findings.find((f) => f.antipattern === "gradient-text");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(8);
  });

  test("flattens a template-literal className so literal slop text still fires", () => {
    const jsx = [
      "export default function Hero({ active }) {",
      "  return (",
      "    <div>",
      "      <h1 class={`bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent ${active ? \"opacity-100\" : \"opacity-0\"}`}>",
      "        Hi",
      "      </h1>",
      "    </div>",
      "  );",
      "}",
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "hero.tsx" });
    const hit = findings.find((f) => f.antipattern === "gradient-text");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(4);
  });

  test("maps a finding to its true line when class={ opens on its own line, above the template literal", () => {
    // Reproduces the confirmed review bug: `class={` on one line, the backtick literal on
    // the NEXT line, and the closing `}` on a line after that. The leading `\s*` padding
    // (between `{` and the backtick) contains a newline that must be restored BEFORE the
    // flattened replacement, not dumped at the tail of the whole span — otherwise the
    // slop text's finding gets misattributed to an earlier line (line 5 instead of 6).
    const jsx = [
      "export default function Hero() {",                                                          // 1
      "  return (",                                                                                 // 2
      "    <div>",                                                                                  // 3
      "      <h1",                                                                                  // 4
      "        class={",                                                                            // 5
      "          `bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent`",      // 6
      "        }",                                                                                   // 7
      "      >",                                                                                     // 8
      "        Hi",                                                                                  // 9
      "      </h1>",                                                                                 // 10
      "    </div>",                                                                                  // 11
      "  );",                                                                                        // 12
      "}",                                                                                           // 13
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "hero.tsx" });
    const hit = findings.find((f) => f.antipattern === "gradient-text");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(6);
  });

  test("maps content after a multi-line dropped ${...} expression to its true trailing line", () => {
    // The expression itself spans lines 4-6; "Acme" lives AFTER the expression's closing
    // `}`, on line 6 — it must not be misattributed to an earlier line. Uses the
    // placeholder-name gate (not gradient-text) because it reports the line of the ACTUAL
    // matched substring, not the line the enclosing class= attribute starts on — the right
    // probe for whether in-value newlines land in the correct place after flattening.
    const jsx = [
      "export default function Hero({ x }) {", // 1
      "  return (",                             // 2
      "    <div>",                              // 3
      "      <h1 class={`start ${",             // 4
      "        x",                              // 5
      "      } Acme`}>",                        // 6
      "        Hi",                             // 7
      "      </h1>",                            // 8
      "    </div>",                             // 9
      "  );",                                   // 10
      "}",                                       // 11
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "hero.tsx" });
    const hit = findings.find((f) => f.antipattern === "placeholder-name");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(6);
  });

  test("does not misinterpret a style={{...}} object as a class attribute", () => {
    const jsx = [
      "export default function Box() {",
      "  return (",
      "    <div style={{ background: \"linear-gradient(90deg, red, blue)\" }}>",
      "      <p>hi</p>",
      "    </div>",
      "  );",
      "}",
    ].join("\n");

    // Should not throw, and should not fire gradient-text (bg-clip-text/text-transparent absent).
    const findings = detectSlopJsx(jsx, { file: "box.tsx" });
    expect(findings.some((f) => f.antipattern === "gradient-text")).toBe(false);
  });

  test("sees slop text inside a cn()-wrapped className (issue #3 regression)", () => {
    // The exact evasion from issue #3: a literal className and a cn()-wrapped one with
    // IDENTICAL classes must fire the same gates the same number of times.
    const literal = [
      "export default function Pill({ live }) {",
      "  return (",
      "    <h1 className=\"bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent\">",
      "      Hi",
      "    </h1>",
      "  );",
      "}",
    ].join("\n");
    const wrapped = [
      "export default function Pill({ live }) {",
      "  return (",
      "    <h1 className={cn(\"bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent\")}>",
      "      Hi",
      "    </h1>",
      "  );",
      "}",
    ].join("\n");

    const literalHits = detectSlopJsx(literal, { file: "pill.tsx" }).filter((f) => f.antipattern === "gradient-text");
    const wrappedHits = detectSlopJsx(wrapped, { file: "pill.tsx" }).filter((f) => f.antipattern === "gradient-text");
    expect(wrappedHits.length).toBe(literalHits.length);
    expect(wrappedHits.length).toBeGreaterThan(0);
    expect(wrappedHits[0].line).toBe(3);
  });

  test("sees a status-dot pill whose container className is a clsx() call with a ternary and a && arm", () => {
    const jsx = [
      "export default function StatusPill({ live }) {",
      "  return (",
      "    <span",
      "      className={clsx(",
      "        \"inline-flex rounded-full\",",
      "        live && \"bg-emerald-50 text-emerald-700\",",
      "        live ? \"animate-pulse\" : \"opacity-50\"",
      "      )}",
      "    >",
      "      <span className=\"h-2 w-2 rounded-full bg-emerald-500\" />",
      "      status",
      "    </span>",
      "  );",
      "}",
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "status.tsx" });
    const hit = findings.find((f) => f.antipattern === "status-dot-pill");
    expect(hit).toBeDefined();
  });

  test("preserves line count / mapping across a multi-line cn() call", () => {
    const jsx = [
      "export default function Card() {",                                                          // 1
      "  return (",                                                                                 // 2
      "    <div",                                                                                   // 3
      "      className={cn(",                                                                       // 4
      "        \"rounded-lg\",",                                                                     // 5
      "        \"bg-gradient-to-r from-purple-500 to-pink-500 bg-clip-text text-transparent\"",      // 6
      "      )}",                                                                                    // 7
      "    >",                                                                                       // 8
      "      Hi",                                                                                    // 9
      "    </div>",                                                                                  // 10
      "  );",                                                                                        // 11
      "}",                                                                                            // 12
    ].join("\n");

    const findings = detectSlopJsx(jsx, { file: "card.tsx" });
    const hit = findings.find((f) => f.antipattern === "gradient-text");
    expect(hit).toBeDefined();
    expect(hit.line).toBe(4);
  });
});
