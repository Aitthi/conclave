// DELIBERATELY BAD — see design-host/evals/thresholds.json's "bad-fixture" target. The
// gate asserts this whole tree STAYS failing (guards the grader against silently going
// permissive). Do not "improve" it.
//
// Expected fails: A1a (theme.css defines only 2 colours), A1b (raw hex hardcoded all over
// instead of token classes), A2 (a lone component imported by only THIS screen — nothing
// shared by ≥2 screens), A5 (gradient-text headline). Nav between the two screens is left
// WORKING (real <Link>) so A3 stays a clean pass on purpose — the fixture proves the OTHER
// checks still discriminate, not that everything fails at once. (A4 is failed by two.tsx.)
import { Link } from "react-router-dom";
import Banner from "../components/Banner";

export const meta = { title: "Step one" };

export default function One() {
  return (
    <section style={{ background: "#ffffff", color: "#111111", padding: 40 }}>
      <h1 className="bg-clip-text text-transparent bg-gradient-to-r from-red-500 to-blue-500" style={{ fontSize: 18 }}>
        <Banner />
      </h1>
      <p style={{ color: "#aaaaaa" }}>Review the box of coffee beans and confirm the order before you continue.</p>
      <div style={{ border: "1px solid #cccccc", padding: 16, background: "#f5f5f5" }}>
        <span style={{ color: "#3b82f6" }}>qty</span>
        <span style={{ color: "#888888" }}>2</span>
      </div>
      <Link to="/two" style={{ color: "#3b82f6" }}>
        Next
      </Link>
    </section>
  );
}
