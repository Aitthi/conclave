export default function Welcome() {
  return (
    <main className="min-h-screen grid place-items-center bg-canvas font-sans">
      <div className="max-w-md px-8 text-center">
        <h1 className="text-2xl font-semibold text-ink">Your canvas is live</h1>
        <p className="mt-3 text-sm leading-relaxed text-muted">
          Ask your agent to design something. Screens are React files in{" "}
          <span className="text-ink">design/screens/</span>, styled with Tailwind
          utilities built from the tokens in{" "}
          <span className="text-ink">design/theme.css</span>.
        </p>
      </div>
    </main>
  );
}
