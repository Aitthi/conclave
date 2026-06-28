// Honest deferred/empty placeholder — a muted card naming what's missing or the
// milestone a section is waiting on. Shared by the Context drawer, Timeline, and
// any other surface that must render an HONEST empty state instead of fabricated
// rows (this repo forbids fake data — a deferred note is the sanctioned filler).
export function DeferredNote({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 text-[11.5px] text-text-tertiary">
      {children}
    </div>
  );
}
