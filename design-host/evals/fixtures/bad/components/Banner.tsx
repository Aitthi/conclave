// DELIBERATELY BAD fixture support file. This component exists ONLY so A2_shared FAILS
// non-vacuously: the port's A2 vacuous-passes when there are 0 components, so a bad
// fixture with no components/ would wrongly PASS A2. Instead we ship ONE component that is
// imported by exactly ONE screen (one.tsx) — so there ARE components and there ARE ≥2
// screens (A2 is not vacuous), yet nothing is shared by ≥2 screens, and A2 fails. Kept
// hex-free so it doesn't perturb the A1b budget. Do not "improve" it.
export default function Banner() {
  return <div>Checkout</div>;
}
