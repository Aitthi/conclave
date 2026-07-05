// DELIBERATELY BAD — see one.tsx's header. A near-identical copy of "one" (no shared
// layout — everything pasted per screen) PLUS a deliberate TSX syntax error (`const total
// = ;`) so A4_render FAILS: the port has no minScreens concept, so A4 is failed here on a
// real parse error that esbuild transformSync catches, not on a screen count. The <Link>
// back to "one" is still valid text, so A3 reachability (a regex over nav targets) is
// unaffected and A3 stays a clean pass. Do not "fix" the syntax error.
import { Link } from "react-router-dom";

export const meta = { title: "Step two" };

export default function Two() {
  const total = ;
  return (
    <section style={{ background: "#ffffff", color: "#111111", padding: 40 }}>
      <h1 className="bg-clip-text text-transparent bg-gradient-to-r from-red-500 to-blue-500" style={{ fontSize: 18 }}>
        Checkout
      </h1>
      <p style={{ color: "#aaaaaa" }}>Review the box of coffee beans and confirm the order before you continue.</p>
      <div style={{ border: "1px solid #cccccc", padding: 16, background: "#f5f5f5" }}>
        <span style={{ color: "#3b82f6" }}>qty</span>
        <span style={{ color: "#888888" }}>2</span>
      </div>
      <Link to="/one" style={{ color: "#3b82f6" }}>
        Back
      </Link>
    </section>
  );
}
