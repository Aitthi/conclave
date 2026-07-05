export const meta = { title: "Home" };

export default function Home() {
  return (
    <main className="min-h-screen grid place-items-center bg-bg text-fg">
      <div className="text-center space-y-3">
        <h1 className="text-4xl font-semibold">Your canvas is live</h1>
        <p className="opacity-70">Ask your agent to design something — screens are React files in .arta/proto/screens/.</p>
      </div>
    </main>
  );
}
