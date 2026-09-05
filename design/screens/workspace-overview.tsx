import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { CalendarDays, ChevronRight, FolderCog, Info, Table2 } from "lucide-react";

export const meta = { title: "Overview — measured AI usage and daily activity" };

type Coverage = "complete" | "partial" | "none";
type Event = {date:string; model:string; agent:string; workspace:string; tokens:number|null};
/* Explicit illustrative fixture records. Never presented as live user telemetry.
   Token numbers below represent input+output only when both components are known.
   Production adapter must consume the pinned usage.overview wire contract. */
const RECORDS:Event[] = [
  {date:"2026-09-02",model:"Claude Sonnet",agent:"Dew",workspace:"codeup",tokens:12400},
  {date:"2026-09-02",model:"Claude Sonnet",agent:"Dew",workspace:"codeup",tokens:8200},
  {date:"2026-09-03",model:"Unknown model",agent:"Aoki",workspace:"codeup",tokens:null},
  {date:"2026-09-04",model:"Claude Sonnet",agent:"Dew",workspace:"codeup",tokens:18600},
  {date:"2026-09-04",model:"Claude Sonnet",agent:"Dew",workspace:"codeup",tokens:null},
  {date:"2026-09-05",model:"GPT-5",agent:"Marty",workspace:"archive-lab",tokens:9600},
];
const CONTEXT = [
  {agent:"Dew",model:"Claude Sonnet",workspace:"codeup",used:42000,limit:200000,source:"Transcript",observed:"5 Sep, 13:20"},
  {agent:"Aoki",model:"Unknown model",workspace:"codeup",used:null,limit:null,source:"Unavailable",observed:"No observation"},
  {agent:"Marty",model:"GPT-5",workspace:"archive-lab",used:36000,limit:200000,source:"Transcript",observed:"4 Sep, 18:10 · stale"},
];
const TODAY = "2026-09-05";
const DAY=86400000;
const fmt=(n:number)=>n.toLocaleString("en-US");
const focus="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";
const pill="rounded-md border border-border bg-surface px-2 py-1.5 text-[11px] "+focus;

export default function UsageOverview() {
  const params=useMemo(()=>new URLSearchParams(location.search),[]);
  const preview=params.get("state")||"default";
  const dark=params.get("theme")==="dark";
  const [range,setRange]=useState(90);
  const [model,setModel]=useState("All models");
  const [agent,setAgent]=useState("All agents");
  const [workspace,setWorkspace]=useState("All workspaces");
  const [group,setGroup]=useState("Model");
  const [selected,setSelected]=useState(89);
  const [table,setTable]=useState(false);
  const [failed,setFailed]=useState(preview==="error");
  const cells=useRef<(HTMLButtonElement|null)[]>([]);
  const loading=preview==="loading";
  const unavailable=preview==="none"||preview==="unsupported";
  const empty=preview==="empty"||preview==="zero";
  const records=unavailable||empty||loading||failed?[]:RECORDS.filter(e=>(model==="All models"||e.model===model)&&(agent==="All agents"||e.agent===agent)&&(workspace==="All workspaces"||e.workspace===workspace));
  const matches=(e:{model:string;agent:string;workspace:string})=>(model==="All models"||e.model===model)&&(agent==="All agents"||e.agent===agent)&&(workspace==="All workspaces"||e.workspace===workspace);
  const dates=Array.from({length:range},(_,i)=>new Date(Date.parse(TODAY+"T00:00:00Z")-(range-i-1)*DAY).toISOString().slice(0,10));
  const coverage=(date:string):Coverage=>preview==="zero"&&date!==TODAY?"complete":unavailable||loading||failed||date<"2026-09-01"?"none":date==="2026-09-03"||date===TODAY?"partial":"complete";
  const buckets=dates.map(date=>{const events=records.filter(e=>e.date===date);return {date,count:events.length,coverage:coverage(date)};});
  const firstDay=new Date(dates[0]+"T00:00:00Z").getUTCDay();
  const columns=Math.ceil((range+firstDay)/7);
  const padded=[...Array(firstDay).fill(null),...buckets,...Array(columns*7-range-firstDay).fill(null)];
  const current=buckets[Math.min(selected,range-1)];
  const exact=(b:typeof current)=>b.coverage==="none"?"Unavailable":b.coverage==="partial"&&b.count===0?"— · partial":fmt(b.count)+(b.coverage==="partial"?" · partial":"");
  const measured=records.reduce((n,e)=>n+(e.tokens??0),0);
  const missing=records.filter(e=>e.tokens===null).length;
  const hasCoverage=buckets.some(b=>b.coverage!=="none");
  const fullyCovered=buckets.every(b=>b.coverage==="complete");
  const rows=Object.values(records.reduce<Record<string,{name:string;count:number;tokens:number;missing:number}>>((out,e)=>{
    const name=group==="Model"?e.model:group==="Agent"?e.agent:e.workspace;
    out[name]??={name,count:0,tokens:0,missing:0};out[name].count++;out[name].tokens+=e.tokens??0;out[name].missing+=e.tokens===null?1:0;return out;
  },{}));
  useEffect(()=>{document.documentElement.classList.toggle("dark",dark);return()=>document.documentElement.classList.remove("dark");},[dark]);

  return <div className="usage-canon flex h-screen min-w-[760px] overflow-hidden bg-canvas font-sans text-text-primary">
    <style>{`
      .usage-canon { --heat-0:#e6e6ed;--heat-1:#c0bbed;--heat-2:#9187dd;--heat-3:#7060c8;--heat-4:#5142ab; }
      .dark .usage-canon { --heat-0:#32323c;--heat-1:#464064;--heat-2:#615395;--heat-3:#7e6dc6;--heat-4:#a595ef; }
      .usage-canon .unknown-cell { border:1px dashed var(--color-text-secondary);background:transparent;opacity:.45; }
      .usage-canon .partial-cell { outline:1px dotted var(--color-text-primary);outline-offset:1px; }
      .usage-canon .sr-only { position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0; }
      .usage-canon .calendar-rows { display:grid;grid-template-rows:repeat(7,16px);gap:6px;padding-top:24px; }
    `}</style>
    <nav className="flex w-14 shrink-0 flex-col items-center gap-3 border-r border-border bg-sidebar py-2" aria-label="Primary">
      <Link to="/workspace-overview" aria-label="Overview" aria-current="page" className={"grid h-9 w-9 place-items-center rounded-lg bg-accent/10 text-accent "+focus}><svg viewBox="0 0 512 512" fill="currentColor" className="h-5 w-5" aria-hidden="true"><path d="M149 97 223 67 322 76 403 133 336 189 292 158 247 152 198 170Z"/><path d="M128 399 76 322 68 256 76 190 128 113 186 179 156 205 152 256 156 307 186 333Z"/><path d="M403 379 322 436 223 445 149 415 198 342 247 360 292 354 336 323Z"/></svg></Link>
      <Link to="/workspace-archive" aria-label="Manage workspaces" className={"grid h-9 w-9 place-items-center rounded-lg text-text-secondary "+focus}><FolderCog className="h-4 w-4"/></Link>
    </nav>
    <main className="min-w-0 flex-1 overflow-auto">
      <header className="flex h-16 items-center justify-between gap-4 border-b border-border bg-surface px-6"><div><h1 className="text-[19px] font-semibold">Overview</h1><p className="mt-0.5 text-[11px] text-text-secondary">Recorded AI activity and measured tokens</p></div><Link to="/workspace-archive" className={pill}>Manage workspaces</Link></header>
      <div className="mx-auto max-w-[1280px] space-y-4 px-6 py-4 max-[980px]:px-5">
        <p className="text-[10px] text-text-secondary">Design specimen · illustrative records · 5 Sep 2026 · Asia/Bangkok</p>
        <div className="flex flex-wrap items-center gap-2">
          {[[model,setModel,["All models","Claude Sonnet","GPT-5","Unknown model"]],[agent,setAgent,["All agents","Dew","Aoki","Marty"]],[workspace,setWorkspace,["All workspaces","codeup","archive-lab"]]].map(([value,set,options],i)=><label key={i}><span className="sr-only">{["Model","Agent","Workspace"][i]}</span><select className={pill} value={value as string} onChange={e=>(set as (v:string)=>void)(e.target.value)}>{(options as string[]).map(v=><option key={v}>{v}</option>)}</select></label>)}
          <label className="ml-auto flex items-center gap-2"><CalendarDays className="h-3.5 w-3.5 text-text-secondary"/><span className="sr-only">Date range</span><select className={pill} value={range} onChange={e=>{setRange(Number(e.target.value));setSelected(Number(e.target.value)-1);}}><option value={30}>Last 30 days</option><option value={90}>Last 90 days</option></select></label>
        </div>
        <p className="text-[10.5px] text-text-secondary">Archived workspace history included · hidden internal workspaces excluded</p>
        {failed?<div role="alert" className="flex items-center justify-between rounded-lg bg-surface p-4 ring-1 ring-border"><span className="text-[12px]">Couldn’t load usage. Measurements are unavailable.</span><button className={pill} onClick={()=>setFailed(false)}>Retry</button></div>:<section aria-label="Period measurements" className="grid grid-cols-3 divide-x divide-border border-y border-border py-3">
          {[
            ["Measured tokens",loading?"Loading…":!hasCoverage?"Unavailable":records.length>missing?fmt(measured):fullyCovered&&!records.length?"0":"—",hasCoverage?missing+" activity records have unknown token usage":"Missing-record count unavailable"],
            ["Recorded activity",loading?"Loading…":!hasCoverage?"Unavailable":records.length?fmt(records.length):fullyCovered?"0":"—","Completed responses / stable usage records"],
            ["Coverage",loading?"Loading…":!hasCoverage?"None":fullyCovered?"Complete":"Partial",hasCoverage?"History starts 1 Sep · gaps remain unknown":"No supported observation in this range"],
          ].map(([label,value,help])=><div key={label} className="px-4 first:pl-0"><h2 className="text-[11px] text-text-secondary">{label}</h2><div className="my-2 font-semibold tabular-nums" style={{fontSize:25}}>{value}</div><p className="text-[10.5px] leading-relaxed text-text-secondary">{help}</p></div>)}
        </section>}
        <p role="status" className="flex items-start gap-2 text-[11px] leading-relaxed text-text-secondary"><Info className="mt-0.5 h-3.5 w-3.5 shrink-0"/>{unavailable?"This source has no validated usage observations. Unknown does not mean zero.":"Measured tokens include known input + output only. They are not total account consumption; missing usage and observation gaps remain visible."}</p>
        <section aria-labelledby="activity-title" className="rounded-xl bg-surface p-4 ring-1 ring-border">
          <div className="flex items-start justify-between"><div><h2 id="activity-title" className="text-[13px] font-semibold">Daily activity</h2><p className="mt-1 text-[10.5px] text-text-secondary">{dates[0]} – {TODAY} · Asia/Bangkok · Today in progress</p></div><button onClick={()=>setTable(v=>!v)} className={"inline-flex items-center gap-1.5 "+pill}><Table2 className="h-3 w-3"/>{table?"Calendar":"Daily table"}</button></div>
          {loading?<div aria-label="Loading daily activity" className="my-5 rounded bg-fill" style={{height:128}}/>:table?<div className="mt-4 max-h-44 overflow-auto"><table className="w-full text-left text-[11px]"><thead><tr><th>Date</th><th>Activity records</th><th>Coverage</th></tr></thead><tbody>{buckets.map(b=><tr key={b.date} className="border-t border-border"><td className="py-1.5">{b.date}{b.date===TODAY?" · in progress":""}</td><td>{exact(b)}</td><td>{b.coverage}</td></tr>)}</tbody></table></div>:<div className="mt-4 flex gap-3 overflow-x-auto pb-2">
            <div className="calendar-rows shrink-0 text-[9px] text-text-secondary">{["Sun","Mon","Tue","Wed","Thu","Fri","Sat"].map(d=><span key={d} className="flex h-4 items-center">{d}</span>)}</div>
            <div><div className="mb-2 grid h-4 text-[10px] text-text-secondary" style={{gridTemplateColumns:`repeat(${columns},22px)`}}>{Array.from({length:columns},(_,i)=>{const date=dates[Math.max(0,i*7-firstDay)];const prev=dates[Math.max(0,(i-1)*7-firstDay)];return <span key={i} className="overflow-visible whitespace-nowrap">{i===columns-1?"Sep":i===0||date?.slice(0,7)!==prev?.slice(0,7)?new Date(date+"T00:00:00Z").toLocaleString("en",{month:"short",timeZone:"UTC"}):""}</span>;})}</div>
              <div role="group" aria-label="Daily recorded activity" className="grid w-max gap-1.5" style={{gridAutoFlow:"column",gridTemplateRows:"repeat(7,16px)",gap:6,gridTemplateColumns:`repeat(${columns},16px)`}}>
                {padded.map((b,i)=>!b?<span key={"outside"+i} aria-hidden="true" className="h-4 w-4"/>:<button key={b.date} ref={el=>{cells.current[i-firstDay]=el;}} tabIndex={i-firstDay===selected?0:-1} title={b.date+" · "+exact(b)+" activity records · "+b.coverage} aria-label={b.date+", "+exact(b)+" activity records, "+b.coverage+(b.date===TODAY?", in progress":"")} onFocus={()=>setSelected(i-firstDay)} onMouseEnter={()=>setSelected(i-firstDay)} onClick={()=>setSelected(i-firstDay)} onKeyDown={e=>{const step=e.key==="ArrowRight"?7:e.key==="ArrowLeft"?-7:e.key==="ArrowDown"?1:e.key==="ArrowUp"?-1:0;if(step){e.preventDefault();e.stopPropagation();const n=Math.max(0,Math.min(range-1,i-firstDay+step));setSelected(n);cells.current[n]?.focus();}}} className={`h-4 w-4 rounded-[3px] ${focus} ${b.coverage==="none"||b.coverage==="partial"&&!b.count?"unknown-cell":""} ${b.coverage==="partial"?"partial-cell":""}`} style={b.coverage==="complete"||b.count?{backgroundColor:`var(--heat-${Math.min(b.count,4)})`}:undefined}/>)}
              </div>
            </div>
            <div className="ml-auto max-w-[230px] self-center pl-5 text-[11px] leading-relaxed text-text-secondary max-[980px]:hidden"><p className="font-medium text-text-primary">History begins with collection</p><p className="mt-2">Earlier dates and observation gaps are unknown. A solid empty cell is a verified zero.</p></div>
          </div>}
          <output aria-live="polite" className="mt-3 block border-t border-border pt-3 text-[11px] text-text-secondary">{current.date} · {exact(current)} activity records · Coverage: {current.coverage}{current.date===TODAY?" · In progress":""}</output>
          <div className="mt-3 flex flex-wrap items-center gap-2 text-[10px] text-text-secondary"><span>Activity records / day</span>{[0,1,2,3,4].map(n=><span key={n} className="inline-flex items-center gap-1"><span className="h-3 w-3 rounded-[3px]" style={{backgroundColor:`var(--heat-${n})`}}/>{n===4?"4+":n}</span>)}<span className="unknown-cell ml-2 h-3 w-3 rounded-[3px]"/><span>Unknown</span><span className="partial-cell ml-2 h-3 w-3 rounded-[3px]"/><span>Partial</span></div>
        </section>
        <section><div className="flex items-center justify-between"><h2 className="text-[13px] font-semibold">Usage breakdown</h2><div className="flex gap-1 rounded-md bg-fill p-1">{["Model","Agent","Workspace"].map(v=><button key={v} aria-pressed={group===v} onClick={()=>setGroup(v)} className={"rounded px-3 py-1 text-[11px] "+focus+" "+(group===v?"bg-surface font-semibold":"text-text-secondary")}>{v}</button>)}</div></div>
          <table className="mt-3 w-full rounded-xl bg-surface text-left text-[11px]"><thead><tr className="border-b border-border text-text-secondary"><th className="px-4 py-3">{group}</th><th>Activity records</th><th>Measured tokens</th><th>Unknown usage</th></tr></thead><tbody>{rows.map(row=><tr key={row.name} className="border-b border-border last:border-0"><td className="px-4 py-3 font-medium">{row.name}{row.name==="archive-lab"&&<span className="ml-2 text-[10px] text-text-secondary">Archived</span>}</td><td>{row.count}</td><td>{row.missing===row.count?"—":fmt(row.tokens)}</td><td>{row.missing} records</td></tr>)}{!rows.length&&<tr><td colSpan={4} className="px-4 py-6 text-text-secondary">{loading?"Loading attribution…":hasCoverage?"No recorded activity matches these filters. Coverage gaps remain unknown.":"Usage unavailable for these filters."}</td></tr>}</tbody></table>
        </section>
        <section className="border-t border-border pt-4"><h2 className="text-[13px] font-semibold">Current context <span className="ml-2 text-[10.5px] font-normal text-text-secondary">Latest snapshots · independent of date range</span></h2><p className="mt-1 text-[10.5px] text-text-secondary">Context is not cumulative usage. Capacities and snapshots are never summed.</p>
          {CONTEXT.filter(matches).map(c=><div key={c.agent} style={{display:"grid",gridTemplateColumns:"100px minmax(0,1fr) 160px",gap:16}} className="mt-3 items-center text-[11px]"><div className="font-medium">{c.agent}<p className="mt-1 text-[10px] font-normal text-text-secondary">{c.workspace}{c.workspace==="archive-lab"?" · Archived":""}</p></div><div>{c.used===null?"Unavailable":<><div className="mb-1.5 tabular-nums">{fmt(c.used)} / {fmt(c.limit!)} tokens · {Math.round(c.used/c.limit!*100)}%</div><meter className="h-2 w-full" min={0} max={c.limit!} value={c.used} aria-label={c.agent+" current context"}/></>}</div><div className="text-[10px] leading-relaxed text-text-secondary">{c.source}<br/>{c.observed}</div></div>)}
        </section>
      </div>
    </main>
  </div>;
}
