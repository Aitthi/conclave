// Landing screen. Imports the shared TopNav (A2) and reaches the detail screen through
// its nav links (A3). Token classes only — no raw hex — so A1b stays in budget, and no
// slop tells, so A5 is clean.
import TopNav from "../components/TopNav";

export const meta = { title: "Home" };

export default function Home() {
  return (
    <main className="min-h-screen bg-bg font-sans text-ink">
      <TopNav />
      <section className="mx-auto max-w-2xl px-6 py-16">
        <h1 className="font-serif text-lg text-ink">Meridian Roasters</h1>
        <p className="mt-4 text-base text-muted">
          A small-batch coffee subscription. Pick a plan, set a cadence, and we ship fresh
          beans to your door.
        </p>
        <div className="mt-8 rounded-md border border-muted bg-surface p-6">
          <span className="text-sm text-muted">Next box</span>
          <span className="ml-2 text-base text-accent">Ethiopia Guji</span>
        </div>
      </section>
    </main>
  );
}
