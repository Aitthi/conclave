import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { CalendarDays, Database, FolderCog, Info } from "lucide-react";

export const meta = { title: "Overview — AI usage and recorded activity" };

/* Provisional geometry only. Metric, timezone, attribution and collection
   coverage await Aoki's research-backed contract. No fabricated measurements. */
type Bucket = { date: string; count: number | null; unit: string | null };
const END = Date.UTC(2026, 8, 5);
const DAY = 86400000;

export default function UsageOverview() {
  const params = useMemo(() => new URLSearchParams(location.search), []);
  const state = params.get("state") || "default";
  const dark = params.get("theme") === "dark";
  const [weeks, setWeeks] = useState(52);
  const [workspace, setWorkspace] = useState("All workspaces");
  const [provider, setProvider] = useState("All providers");
  const [group, setGroup] = useState("Model");
  const [selected, setSelected] = useState(0);
  const [table, setTable] = useState(false);
  const cells = useRef<(HTMLButtonElement | null)[]>([]);
  const buckets = useMemo<Bucket[]>(() => Array.from({ length: weeks * 7 }, (_, i) => ({
    date: new Date(END - (weeks * 7 - i - 1) * DAY).toISOString().slice(0, 10),
    count: null, unit: null,
  })), [weeks]);
  const current = buckets[Math.min(selected, buckets.length - 1)];
  const loading = state === "loading";
  const error = state === "error";
  const explanation = state === "unsupported"
    ? "Usage measurement is unavailable for this provider."
    : state === "partial"
      ? "Some sources or periods are missing. Only measured usage can contribute to totals."
      : "No recorded usage yet. Earlier activity may not be available when collection begins.";

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    return () => document.documentElement.classList.remove("dark");
  }, [dark]);

  return <div className="usage-canon flex h-screen min-w-[760px] overflow-hidden bg-canvas font-sans text-text-primary">
    <style>{`
      .usage-canon { --heat-0: #e8e8ef; --heat-1: #c4c0f1; --heat-2: #9b92e5; --heat-3: #7468d5; --heat-4: #5143b6; }
      .dark .usage-canon { --heat-0: #292932; --heat-1: #373251; --heat-2: #514887; --heat-3: #6b5bb5; --heat-4: #8d7bea; }
      .usage-canon .heat-unknown { background: transparent; border: 1px dashed var(--color-border); }
      .usage-canon button:focus-visible, .usage-canon a:focus-visible, .usage-canon select:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 3px; }
    `}</style>
    <nav aria-label="Primary" className="flex w-14 shrink-0 flex-col items-center gap-3 border-r border-border bg-sidebar py-2">
      <Link to="/workspace-overview" aria-label="Overview" aria-current="page" className="grid h-9 w-9 place-items-center rounded-[9px] bg-accent/10 text-accent">
        <svg viewBox="0 0 512 512" className="h-5 w-5" fill="currentColor" aria-hidden="true">
          <path d="M149 97 223 67 322 76 403 133 336 189 292 158 247 152 198 170Z" />
          <path d="M128 399 76 322 68 256 76 190 128 113 186 179 156 205 152 256 156 307 186 333Z" />
          <path d="M403 379 322 436 223 445 149 415 198 342 247 360 292 354 336 323Z" />
        </svg>
      </Link>
      <Link to="/workspace-archive" aria-label="Manage workspaces" title="Manage workspaces" className="grid h-9 w-9 place-items-center rounded-lg text-text-secondary hover:bg-fill"><FolderCog className="h-[18px] w-[18px]" /></Link>
    </nav>
    <main className="min-w-0 flex-1 overflow-auto">
      <header className="flex h-16 items-center justify-between gap-4 border-b border-border bg-surface px-7">
        <div><h1 className="text-[19px] font-semibold tracking-[-0.02em]">Overview</h1><p className="mt-0.5 text-[11px] text-text-secondary">AI usage across your workspaces</p></div>
        <Link to="/workspace-archive" className="inline-flex items-center gap-2 rounded-md bg-fill px-3 py-2 text-[11px] font-medium"><FolderCog className="h-3.5 w-3.5" />Manage workspaces</Link>
      </header>
      <div className="mx-auto max-w-[1320px] space-y-5 px-7 py-5 max-[980px]:px-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex gap-2">
            <label><span className="sr-only">Workspace</span><select value={workspace} onChange={e => setWorkspace(e.target.value)} className="h-8 rounded-md border border-border bg-surface px-2 text-[11px]"><option>All workspaces</option><option>codeup</option><option>launchpad</option></select></label>
            <label><span className="sr-only">Provider</span><select value={provider} onChange={e => setProvider(e.target.value)} className="h-8 rounded-md border border-border bg-surface px-2 text-[11px]"><option>All providers</option><option>Codex</option><option>Claude Code</option><option>Antigravity</option></select></label>
          </div>
          <label className="inline-flex items-center gap-2 text-[11px]"><CalendarDays className="h-3.5 w-3.5" /><span className="sr-only">Date range</span><select value={weeks} onChange={e => { setWeeks(Number(e.target.value)); setSelected(0); }} className="h-8 rounded-md border border-border bg-surface px-2"><option value={4}>4 weeks</option><option value={13}>13 weeks</option><option value={26}>26 weeks</option><option value={52}>52 weeks</option></select></label>
        </div>
        <section aria-label="Token measurements" className="grid grid-cols-3 divide-x divide-border border-y border-border py-4">
          {[
            ["Period token usage", "Measured tokens in the selected range"],
            ["Current context", "Latest observed context per agent"],
            ["Measurement coverage", "Sources and periods with recorded data"],
          ].map(([label, copy]) => <div key={label} className="px-4 first:pl-0"><h2 className="text-[11px] font-medium text-text-secondary">{label}</h2>
            {loading ? <div className="my-3 h-5 w-24 rounded bg-fill" aria-label="Loading" /> : <div className="my-2 text-[22px] font-semibold">—<span className="ml-2 text-[11px] font-normal text-text-secondary">Unavailable</span></div>}
            <p className="text-[10.5px] text-text-secondary">{copy}</p></div>)}
        </section>
        <div role={error ? "alert" : "status"} className="flex items-start gap-2 text-[11px] leading-relaxed text-text-secondary"><Info className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span>{error ? "Couldn’t load usage. Measurements remain unavailable until refresh succeeds." : loading ? "Loading measurements and coverage…" : explanation}</span>{error && <button onClick={() => location.reload()} className="ml-auto font-semibold text-accent">Retry</button>}</div>

        <section aria-labelledby="activity-title" className="rounded-xl bg-surface p-4 ring-1 ring-border">
          <div className="flex items-start justify-between gap-3"><div><h2 id="activity-title" className="text-[13px] font-semibold">Activity</h2><p className="mt-1 text-[10.5px] text-text-secondary">{buckets[0].date} – {buckets[buckets.length - 1].date} · Daily calendar preview</p></div><button onClick={() => setTable(v => !v)} aria-expanded={table} className="rounded-md bg-fill px-2.5 py-1.5 text-[10.5px] font-medium">{table ? "Show heatmap" : "View dates as table"}</button></div>
          <p className="mt-3 text-[10.5px] text-text-secondary">No measured activity is available for this period. Outlined cells mean unknown.</p>
          {table ? <div className="mt-4 max-h-40 overflow-auto"><table className="w-full text-left text-[11px]"><thead><tr><th className="py-2">Date</th><th>Metric</th><th>Exact count</th><th>Coverage</th></tr></thead><tbody>{buckets.map(b => <tr key={b.date} className="border-t border-border"><td className="py-1.5">{b.date}</td><td>Unavailable</td><td>—</td><td>Unknown</td></tr>)}</tbody></table></div>
          : <div className="mt-4 flex gap-3 overflow-x-auto pb-2">
            <div className="grid shrink-0 grid-rows-7 gap-1 pt-6 text-[9px] text-text-secondary">{["Sun","Mon","Tue","Wed","Thu","Fri","Sat"].map(day => <span key={day} className="flex h-3.5 items-center">{day}</span>)}</div>
            <div className="min-w-0 flex-1">
              <div className="mb-2 flex justify-between text-[10px] text-text-secondary"><span>{buckets[0].date}</span><span>Sep 2026</span></div>
              <div role="group" aria-label="Daily activity: dates currently unknown" className="grid w-max grid-flow-col grid-rows-7 gap-1" style={{ gridTemplateColumns: `repeat(${weeks}, 14px)` }}>
                {buckets.map((b, i) => <button key={b.date} ref={el => {cells.current[i] = el;}} tabIndex={i === selected ? 0 : -1} title={`${b.date} · Activity unavailable · Exact count unknown`} aria-label={`${b.date}, activity unavailable, count unknown`} onMouseEnter={() => setSelected(i)} onFocus={() => setSelected(i)} onClick={() => setSelected(i)} onKeyDown={e => {
                  const step = e.key === "ArrowRight" ? 7 : e.key === "ArrowLeft" ? -7 : e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
                  if (step) { e.preventDefault(); const next = Math.max(0, Math.min(buckets.length - 1, i + step)); setSelected(next); cells.current[next]?.focus(); }
                }} className="heat-unknown h-3.5 min-w-3.5 rounded-[3px]" />)}
              </div>
            </div>
          </div>}
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-3 text-[10px] text-text-secondary">
            <output aria-live="polite">{current.date} · Activity unavailable · Count unknown</output>
            <div className="flex items-center gap-1.5" aria-label="Intensity scale preview"><span>Less</span>{[0,1,2,3,4].map(level => <span key={level} className="h-3 w-3 rounded-[3px]" style={{ backgroundColor: `var(--heat-${level})` }} />)}<span>More</span><span className="heat-unknown ml-2 h-3 w-3 rounded-[3px]" /><span>Unknown</span></div>
          </div>
          <p className="mt-2 text-[10px] text-text-secondary">Scale preview only. Measured unit and bucket definition await the data contract.</p>
        </section>

        <section aria-label="Usage attribution">
          <div className="flex items-center justify-between"><h2 className="text-[13px] font-semibold">Usage breakdown</h2><div className="flex rounded-md bg-fill p-0.5">{["Model", "Agent", "Workspace"].map(value => <button key={value} onClick={() => setGroup(value)} aria-pressed={group === value} className={`rounded px-3 py-1 text-[11px] ${group === value ? "bg-surface font-semibold text-text-primary" : "text-text-secondary"}`}>{value}</button>)}</div></div>
          <div className="mt-3 overflow-hidden rounded-xl bg-surface ring-1 ring-border"><div className="grid grid-cols-[1.4fr_1fr_1fr_1fr] gap-3 border-b border-border px-4 py-2 text-[10.5px] font-medium text-text-secondary"><span>{group}</span><span>Period tokens</span><span>Current context</span><span>Coverage / source</span></div><div className="flex min-h-28 items-center justify-center gap-3 px-4 py-5"><Database className="h-5 w-5 text-text-secondary" /><div><p className="text-[12px] font-medium">{loading ? "Loading attribution…" : "No measured usage to attribute"}</p><p className="mt-1 text-[11px] text-text-secondary">{workspace} · {provider}. Missing measurements remain unknown.</p></div></div></div>
        </section>
        <p className="text-[10.5px] text-text-secondary">Current context is a latest snapshot. It is not added to period token usage.</p>
      </div>
    </main>
  </div>;
}
