import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Archive, ArrowRight, CheckCircle2, CirclePause, FolderPlus, MoreHorizontal, RotateCcw, Search, Square, Trash2, X } from "lucide-react";

export const meta = { title: "Workspaces — Archive, restore and settings" };
type Workspace = { id: string; name: string; path: string; started: boolean; archived: boolean };
const INITIAL: Workspace[] = [
  { id: "codeup", name: "codeup", path: "/Users/detoro/code/codeup", started: true, archived: false },
  { id: "northstar", name: "northstar", path: "/Users/detoro/code/northstar", started: false, archived: false },
  { id: "sandbox", name: "sandbox-lab", path: "/Users/detoro/code/sandbox-lab", started: false, archived: true },
  { id: "notes", name: "old-notes", path: "/Users/detoro/code/old-notes", started: false, archived: true },
];
const focus = "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";
const neutral = "rounded-md bg-fill px-3 py-2 text-[12px] font-medium hover:bg-overlay/[0.08] disabled:opacity-45 disabled:cursor-not-allowed " + focus;
const primary = "rounded-md bg-accent px-3 py-2 text-[12px] font-semibold text-white hover:bg-accent-hover disabled:opacity-50 " + focus;

function Mark() {
  return <svg viewBox="0 0 512 512" className="h-5 w-5" fill="currentColor" aria-hidden="true"><path d="M149 97 223 67 322 76 403 133 336 189 292 158 247 152 198 170Z" /><path d="M128 399 76 322 68 256 76 190 128 113 186 179 156 205 152 256 156 307 186 333Z" /><path d="M403 379 322 436 223 445 149 415 198 342 247 360 292 354 336 323Z" /></svg>;
}

export default function WorkspaceArchive() {
  const params = useMemo(() => new URLSearchParams(location.search), []);
  const preview = params.get("state") || "workspaces";
  const dark = params.get("theme") === "dark";
  const settingsState = preview.startsWith("settings-") || preview === "archive-pending" || preview === "archive-error";
  const [rows, setRows] = useState<Workspace[]>(() => preview === "empty" ? [] : preview === "all-archived" ? INITIAL.map(row => ({...row, started:false, archived:true})) : INITIAL);
  const [tab, setTab] = useState(preview === "archived" || preview.startsWith("restore") || preview === "archive-empty" ? "archived" : "workspaces");
  const [query, setQuery] = useState(preview === "search-empty" ? "release train" : "");
  const [editId, setEditId] = useState<string | null>(settingsState ? preview === "settings-started" ? "codeup" : "northstar" : null);
  const [pending, setPending] = useState<string | null>(preview === "restore-pending" ? "sandbox" : preview === "archive-pending" ? "northstar" : null);
  const [failure, setFailure] = useState<string | null>(preview === "restore-error" ? "sandbox" : preview === "archive-error" ? "northstar" : null);
  const [menu, setMenu] = useState<string | null>(null);
  const [stopConfirm, setStopConfirm] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const [notice, setNotice] = useState<{text:string; undo?:string; open?:string} | null>(preview === "restored" ? {text:"sandbox-lab restored. It remains stopped.",open:"sandbox"} : null);
  const dialog = useRef<HTMLDialogElement>(null);
  const edit = rows.find(row => row.id === editId);
  const [draftName, setDraftName] = useState("");
  const [opened, setOpened] = useState<string | null>(null);
  const active = rows.filter(row => !row.archived);
  const archived = preview === "archive-empty" ? [] : rows.filter(row => row.archived);
  const selectedRows = tab === "archived" ? archived : active;
  const filtered = selectedRows.filter(row => (row.name+" "+row.path).toLowerCase().includes(query.toLowerCase()));
  const loading = preview === "loading";
  const error = preview === "error";

  useEffect(() => { document.documentElement.classList.toggle("dark", dark); return () => document.documentElement.classList.remove("dark"); }, [dark]);
  useEffect(() => {
    if (edit) { setDraftName(edit.name); dialog.current?.showModal(); }
    else dialog.current?.close();
  }, [editId]);
  useEffect(() => {
    if (preview === "restored") setRows(current => current.map(row => row.id === "sandbox" ? {...row,archived:false,started:false} : row));
  }, []);

  function notify(text:string) { setNotice({text}); }
  function closeSettings() { setEditId(null); setStopConfirm(false); setDeleteConfirm(false); }
  function mutate(row:Workspace, archive:boolean) {
    if (pending) return;
    if (archive && row.started) {setFailure(row.id); return;}
    setPending(row.id); setFailure(null);
    window.setTimeout(() => {
      if (preview === "settings-busy" && archive) {setPending(null); setFailure(row.id); return;}
      setRows(current => current.map(item => item.id === row.id ? {...item,archived:archive,started:false} : item));
      setPending(null); setMenu(null); closeSettings();
      setNotice(archive ? {text:row.name+" archived. All records and files are retained.",undo:row.id} : {text:row.name+" restored. It remains stopped.",open:row.id});
    }, 450);
  }
  function openWorkspace(row:Workspace) {
    if (row.archived) return;
    setOpened(row.id);
    notify("Prototype navigation target: "+row.name+". Opening does not start agents.");
  }

  return <div className="archive-canon flex h-screen min-w-[760px] overflow-hidden bg-canvas font-sans text-text-primary">
    <style>{`.archive-canon { --color-danger: #b42318; } .dark .archive-canon { --color-danger: #ff8a80; } .archive-canon [role="alert"].text-danger { background:color-mix(in srgb,var(--color-danger) 10%,var(--color-surface)); }`}</style>
    <nav aria-label="Primary" className="flex w-14 shrink-0 flex-col items-center gap-2 border-r border-border bg-sidebar py-2">
      <Link to="/workspace-overview" aria-label="Overview" title="AI usage Overview" className={"grid h-9 w-9 place-items-center rounded-lg text-text-secondary "+focus}><Mark /></Link>
      {active.map(row => <button key={row.id} onClick={() => openWorkspace(row)} aria-label={"Open "+row.name} title={row.name} className={"relative mt-1 grid h-9 w-9 place-items-center rounded-[10px] bg-accent/10 text-[13px] font-semibold text-accent "+focus}>{row.name[0].toUpperCase()}{!row.started && <CirclePause className="absolute -bottom-1 -right-1 h-3.5 w-3.5 rounded-full bg-sidebar text-text-secondary" />}</button>)}
    </nav>
    <main className="min-w-0 flex-1 overflow-auto">
      <header className="flex h-16 items-center justify-between border-b border-border bg-surface px-7">
        <div><h1 className="text-[19px] font-semibold">Workspaces</h1><p className="mt-0.5 text-[11px] text-text-secondary">Manage project folders and retained workspaces</p></div>
        <button onClick={() => notify("Prototype target: existing Link folder flow.")} className={"inline-flex items-center gap-1.5 "+primary}><FolderPlus className="h-3.5 w-3.5" />New workspace</button>
      </header>
      <div className="mx-auto max-w-[1200px] p-6 max-[980px]:p-5">
        <div className="flex items-end justify-between gap-4">
          <div><div aria-label="Workspace filters" className="flex gap-1">{["workspaces","archived"].map(value => <button key={value} onClick={() => {setTab(value);setMenu(null);}} aria-pressed={tab === value} className={"rounded-md px-3 py-2 text-[12px] "+focus+" "+(tab === value ? "bg-accent/10 font-semibold text-accent" : "text-text-secondary hover:bg-fill")}>{value === "workspaces" ? "Workspaces" : "Archived"} <span className="ml-1">{loading || error ? "—" : value === "workspaces" ? active.length : archived.length}</span></button>)}</div><p className="mt-2 text-[11px] text-text-secondary">{tab === "workspaces" ? "Includes started and stopped workspaces." : "Restore brings a workspace back stopped. Nothing launches."}</p></div>
          <label className="relative"><span className="sr-only">Search name or path</span><Search className="absolute left-2.5 top-2.5 h-3.5 w-3.5 text-text-secondary" /><input value={query} onChange={e => setQuery(e.target.value)} placeholder="Search name or path" className="h-9 w-52 rounded-md border border-border bg-surface pl-8 pr-3 text-[11.5px] placeholder:text-text-secondary focus:outline-none focus:ring-2 focus:ring-accent" /></label>
        </div>
        <section aria-label={tab === "workspaces" ? "Workspaces" : "Archived workspaces"} className="mt-4 rounded-xl bg-surface ring-1 ring-border">
          <div className="grid grid-cols-[minmax(250px,1fr)_110px_160px] gap-4 px-4 py-3 text-[11px] font-medium text-text-secondary"><span>Workspace / folder</span><span>Status</span><span className="text-right">Actions</span></div>
          {loading ? <div aria-busy="true" aria-label="Loading workspaces" className="space-y-5 border-t border-border p-5">{[1,2,3].map(i => <div key={i} className="h-10 rounded bg-fill" />)}</div>
          : error ? <div role="alert" className="border-t border-border px-6 py-14 text-center"><h2 className="text-[14px] font-semibold">Couldn’t load workspaces</h2><p className="mt-2 text-[12px] text-text-secondary">Existing workspaces and running agents are unchanged.</p><button onClick={() => location.reload()} className={"mt-4 "+neutral}>Retry</button></div>
          : !filtered.length ? <div className="border-t border-border px-6 py-14 text-center"><Archive className="mx-auto h-6 w-6 text-text-secondary" /><h2 className="mt-3 text-[15px] font-semibold">{query ? "No matching workspaces" : tab === "archived" ? "No archived workspaces" : archived.length ? "All workspaces are archived" : "Link your first workspace"}</h2><p className="mx-auto mt-2 max-w-md text-[12px] leading-relaxed text-text-secondary">{query ? "Try another name or folder path." : tab === "archived" ? "Archive keeps agents, sessions, tasks, memory, artifacts and project files." : "Link a project folder or restore a workspace to return it to the Rail."}</p><button onClick={() => query ? setQuery("") : tab === "archived" ? setTab("workspaces") : archived.length ? setTab("archived") : notify("Prototype target: Link folder.")} className={"mt-4 "+neutral}>{query ? "Clear search" : tab === "archived" ? "Back to Workspaces" : archived.length ? "View archived" : "Link folder"}</button></div>
          : filtered.map(row => <div key={row.id} className="border-t border-border">
            <div className="grid min-h-20 grid-cols-[minmax(250px,1fr)_110px_160px] items-center gap-4 px-4">
              <div className="flex min-w-0 items-center gap-3"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-fill text-[13px] font-semibold">{row.name[0].toUpperCase()}</span><div className="min-w-0"><h2 className="truncate text-[13px] font-semibold">{row.name}</h2><p title={row.path} className="mt-1 truncate font-mono text-[10.5px] text-text-secondary">{row.path}</p></div></div>
              <span className="inline-flex items-center gap-1.5 text-[11px] text-text-secondary">{row.archived ? <Archive className="h-3.5 w-3.5" /> : row.started ? <Square className="h-3 w-3" /> : <CirclePause className="h-3.5 w-3.5" />}{row.archived ? "Archived" : row.started ? "Started" : "Stopped"}</span>
              <div className="flex justify-end gap-2"><button disabled={pending === row.id} onClick={() => row.archived ? mutate(row,false) : openWorkspace(row)} className={row.archived ? primary : neutral}>{pending === row.id ? "Restoring…" : row.archived ? "Restore" : "Open"}</button><button aria-label={"Manage "+row.name} aria-expanded={menu === row.id} onClick={() => setMenu(menu === row.id ? null : row.id)} className={"rounded-md px-1.5 hover:bg-fill "+focus}><MoreHorizontal className="h-4 w-4" /></button></div>
            </div>
            {failure === row.id && <div role="alert" className="mx-4 mb-3 rounded-md bg-danger/10 p-3 text-[11.5px] text-danger">{row.archived ? "Couldn’t restore. This workspace remains archived." : "Couldn’t archive. The workspace has live or busy work. Wait for it to finish and try again."}<button onClick={() => mutate(row,!row.archived)} className="ml-3 font-semibold underline">Retry</button></div>}
            {menu === row.id && <div className="flex items-center justify-between gap-3 bg-fill px-4 py-3 text-[11.5px]"><span className="text-text-secondary">{row.archived ? "Restore before editing or opening. Permanent deletion is separate." : row.started ? "Stop workspace before archiving, even when no agents are working." : "Archive retains all records and project files."}</span><button onClick={() => {setEditId(row.id);setMenu(null);}} className={neutral}>Manage workspace</button></div>}
          </div>)}
        </section>
        <p className="mt-4 text-[11px] leading-relaxed text-text-secondary">Archived workspaces leave the normal Rail and list. Agents, sessions, tasks, memory, artifacts and project files are retained.</p>
        {opened && <div role="status" className="mt-4 rounded-lg bg-fill p-3 text-[12px]">Workspace destination: {rows.find(row => row.id === opened)?.name}. Runtime state is unchanged.</div>}
      </div>
    </main>

    <dialog ref={dialog} aria-label="Manage workspace" onCancel={e => {if (pending) e.preventDefault(); else closeSettings();}} onClose={() => setEditId(null)} className="m-auto w-[500px] max-w-[calc(100vw-40px)] max-h-[calc(100vh-40px)] overflow-hidden rounded-[14px] bg-surface text-text-primary shadow-xl backdrop:bg-black/40">
      {edit && <div className="flex max-h-[calc(100vh-40px)] flex-col">
        <header className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3"><h2 className="text-[14px] font-semibold">{edit.archived ? "Archived workspace" : "Edit workspace"}</h2><button disabled={!!pending} onClick={closeSettings} aria-label="Close workspace settings" className={focus}><X className="h-4 w-4" /></button></header>
        <div className="min-h-0 overflow-auto p-5">
          <p className="truncate font-mono text-[11px] text-text-secondary" title={edit.path}>{edit.path}</p>
          <label className="mt-4 block text-[12px] font-medium">Name<input disabled={edit.archived || !!pending} value={draftName} onChange={e => setDraftName(e.target.value)} className="mt-2 h-9 w-full rounded-md border border-border bg-fill px-3 text-[13px] disabled:opacity-60 focus:outline-none focus:ring-2 focus:ring-accent" /></label>
          <section className="mt-5 border-t border-border pt-4"><h3 className="flex items-center gap-2 text-[13px] font-semibold"><Archive className="h-4 w-4" />{edit.archived ? "Restore workspace" : "Archive workspace"}</h3><p className="mt-2 text-[12px] leading-relaxed text-text-secondary">{edit.archived ? "Restore to edit or open this workspace. It returns stopped and no agents launch." : "Hide this workspace from the Rail and normal list. All agents, sessions, tasks, memory, artifacts and project files stay."}</p>
            {edit.started ? <div className="mt-3 rounded-lg bg-fill p-3"><p className="text-[12px] font-medium">Stop workspace before archiving.</p><p className="mt-1 text-[11.5px] text-text-secondary">This workspace is started. Archiving never stops agents automatically.</p>
              {stopConfirm && <p role="alert" className="mt-3 text-[12px] leading-relaxed">Stop all live runtimes and their current work? Saved records remain. Archive will still require a separate action.</p>}
              <div className="mt-3 flex gap-2"><button onClick={() => {if (!stopConfirm) setStopConfirm(true); else {setRows(current => current.map(row => row.id === edit.id ? {...row,started:false} : row));setStopConfirm(false);notify(edit.name+" stopped. You can now archive separately.");}}} className={neutral}>{stopConfirm ? "Confirm stop" : "Stop workspace"}</button>{stopConfirm && <button onClick={() => setStopConfirm(false)} className={neutral}>Cancel</button>}<button disabled className={primary}>Archive</button></div>
            </div> : <div className="mt-3"><p className="mb-3 text-[11.5px] text-text-secondary">{edit.archived ? "Archived · records retained" : preview === "settings-busy" ? "Background work is active. Wait for it to finish before archiving." : "Workspace stopped. Archive also checks for live or busy work."}</p><button disabled={!!pending || preview === "settings-busy"} onClick={() => mutate(edit,!edit.archived)} className={neutral}>{pending === edit.id ? "Archiving…" : edit.archived ? "Restore workspace" : "Archive workspace"}</button></div>}
            {failure === edit.id && <p role="alert" className="mt-3 text-[12px] leading-relaxed text-danger">Couldn’t archive: live or busy work was detected. The workspace remains visible. Wait for it to finish, then try again.</p>}
          </section>
          <section className="mt-5 border-t border-border pt-4"><h3 className="text-[13px] font-semibold text-danger">Permanently delete workspace</h3><p className="mt-2 text-[12px] leading-relaxed text-text-secondary">Removes this workspace and its Conclave records. This cannot be undone.</p>{deleteConfirm && <p role="alert" className="mt-3 text-[12px] font-medium">Permanently delete {edit.name} and its Conclave records?</p>}<div className="mt-3 flex gap-2"><button disabled={!!pending} onClick={() => {if (!deleteConfirm) setDeleteConfirm(true); else {setRows(current => current.filter(row => row.id !== edit.id));closeSettings();notify("Prototype workspace deleted.");}}} className={"inline-flex items-center gap-2 rounded-md bg-danger/10 px-3 py-2 text-[12px] font-semibold text-danger "+focus}><Trash2 className="h-3.5 w-3.5" />{deleteConfirm ? "Confirm permanent delete" : "Delete…"}</button>{deleteConfirm && <button onClick={() => setDeleteConfirm(false)} className={neutral}>Cancel</button>}</div></section>
        </div>
        <footer className="flex shrink-0 justify-end gap-2 border-t border-border px-5 py-3"><button disabled={!!pending} onClick={closeSettings} className={neutral}>Cancel</button>{!edit.archived && <button disabled={!!pending || !draftName.trim()} onClick={() => {setRows(current => current.map(row => row.id === edit.id ? {...row,name:draftName.trim()} : row));closeSettings();}} className={primary}>Save changes</button>}</footer>
      </div>}
    </dialog>
    {notice && <div role="status" className="fixed bottom-5 right-5 z-40 flex max-w-[440px] items-center gap-3 rounded-lg bg-surface-raised p-4 text-[12px] shadow-lg"><CheckCircle2 className="h-4 w-4 shrink-0 text-accent" /><span>{notice.text}</span>{notice.undo && <button onClick={() => {const row=rows.find(item => item.id === notice.undo);if(row)mutate(row,false);}} className={"font-semibold text-accent "+focus}>Undo</button>}{notice.open && <button onClick={() => {const row=rows.find(item => item.id === notice.open);if(row)openWorkspace(row);}} className={"font-semibold text-accent "+focus}>Open</button>}<button onClick={() => setNotice(null)} aria-label="Dismiss" className={focus}><X className="h-3.5 w-3.5" /></button></div>}
  </div>;
}
