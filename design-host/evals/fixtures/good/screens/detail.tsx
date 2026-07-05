// Detail screen. Also imports the shared TopNav (so the component is used by ≥2 screens →
// A2) and is reachable from Home via nav (A3). Token classes only; no slop.
import TopNav from "../components/TopNav";

export const meta = { title: "Detail" };

export default function Detail() {
  return (
    <main className="min-h-screen bg-bg font-sans text-ink">
      <TopNav />
      <section className="mx-auto max-w-2xl px-6 py-16">
        <h1 className="font-serif text-lg text-ink">Ethiopia Guji</h1>
        <p className="mt-4 text-base text-muted">
          Washed process, bright and floral, with notes of bergamot and stone fruit.
          Roasted the day before it ships.
        </p>
        <dl className="mt-8 grid grid-cols-2 gap-4 rounded-md border border-muted bg-surface p-6 text-sm">
          <dt className="text-muted">Roast</dt>
          <dd className="text-ink">Light</dd>
          <dt className="text-muted">Cadence</dt>
          <dd className="text-ink">Biweekly</dd>
        </dl>
      </section>
    </main>
  );
}
