import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Layers,
  Search,
  Plus,
  Eye,
  Pencil,
  Lock,
  X,
  ChevronRight,
  ChevronDown,
} from "lucide-react";
import { ipc } from "../ipc";
import type { BlackboardEntry, BlackboardActivity } from "../ipc";
import { timeHint } from "../lib/timeHint";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BlackboardProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

// Resolved writer/agent identity (instanceId → { name, color }).
interface AgentIdentity {
  name: string;
  color: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

// One-line preview of a stored value. `undefined` value → "—". Best-effort
// JSON.stringify; circular / unserialisable values fall back to String().
function previewValue(value: unknown): string {
  if (value === undefined) return "—";
  try {
    const s = JSON.stringify(value);
    return s ?? String(value);
  } catch {
    return String(value);
  }
}

// Pretty-printed full value for the expanded detail block.
function prettyValue(value: unknown): string {
  if (value === undefined) return "—";
  try {
    const s = JSON.stringify(value, null, 2);
    return s ?? String(value);
  } catch {
    return String(value);
  }
}

// Small colored avatar chip (first letter on the agent color).
function WriterAvatar({ identity }: { identity: AgentIdentity }) {
  return (
    <span
      className="w-4 h-4 rounded-[5px] text-white grid place-items-center text-[9px] font-bold shrink-0"
      style={{ backgroundColor: identity.color }}
    >
      {identity.name.charAt(0).toUpperCase()}
    </span>
  );
}

// ── Blackboard ────────────────────────────────────────────────────────────────

export function Blackboard({ workspaceId, workspaceName, onClose }: BlackboardProps) {
  const [entries, setEntries] = useState<BlackboardEntry[]>([]);
  const [activity, setActivity] = useState<BlackboardActivity[]>([]);
  // instanceId → { name, color }, resolved from instance.list ⨝ agentDef.list.
  const [agents, setAgents] = useState<Map<string, AgentIdentity>>(new Map());
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);

  const [search, setSearch] = useState("");
  const [expandedKey, setExpandedKey] = useState<string | null>(null);

  // New-key form state.
  const [showForm, setShowForm] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Fetch blackboard + the agent identity map in parallel. Used on mount and as
  // a manual refresh after a successful write. A real fetch failure (DB down,
  // or plain `vite` dev with no Tauri shell) lands in catch → surfaced as an
  // error, distinct from a genuinely empty blackboard.
  //
  // Memoised per workspaceId so the effect deps stay honest. A monotonic seq
  // guard drops stale responses (StrictMode's mount→cleanup→mount races two
  // loads; a write→refetch can also overlap) — only the latest load wins.
  const seq = useRef(0);
  const load = useCallback(async () => {
    const mine = ++seq.current;
    setLoading(true);
    setLoadError(false);
    try {
      const [bb, instances, defs] = await Promise.all([
        ipc.blackboard.list({ workspaceId }),
        ipc.instance.list({ workspaceId }),
        ipc.agentDef.list(),
      ]);
      const defById = new Map(defs.map((d) => [d.id, d]));
      const map = new Map<string, AgentIdentity>();
      for (const inst of instances) {
        const def = defById.get(inst.agentDefId);
        if (!def) continue;
        map.set(inst.id, { name: def.name, color: def.color ?? "#6e6e73" });
      }
      if (!mounted.current || mine !== seq.current) return;
      setEntries(bb.entries);
      setActivity(bb.activity);
      setAgents(map);
      setLoading(false);
    } catch (err: unknown) {
      if (import.meta.env.DEV) {
        console.error("Blackboard: blackboard.list / instance.list / agentDef.list failed", err);
      }
      if (!mounted.current || mine !== seq.current) return;
      setEntries([]);
      setActivity([]);
      setAgents(new Map());
      setLoading(false);
      setLoadError(true);
    }
  }, [workspaceId]);

  // Load on mount / when the workspace changes (the component is also keyed by
  // workspaceId at the call site, so this typically fires via remount).
  useEffect(() => {
    void load();
  }, [load]);

  async function handleCreate() {
    const key = newKey.trim();
    if (!key) {
      setFormError("Key is required");
      return;
    }
    // Try JSON; if it doesn't parse, store the raw string (so users can type
    // either valid JSON or a plain string). An empty value → empty string.
    let value: unknown = newValue;
    const trimmed = newValue.trim();
    if (trimmed.length > 0) {
      try {
        value = JSON.parse(trimmed);
      } catch {
        value = newValue;
      }
    }
    setSaving(true);
    setFormError(null);
    try {
      // USER write → no writerId, so lastWriterId stays null (unattributed).
      await ipc.blackboard.set({ workspaceId, key, value });
      if (!mounted.current) return;
      setNewKey("");
      setNewValue("");
      setShowForm(false);
      // The write is done — clear `saving` before the refetch so reopening the
      // form doesn't show a stale "saving…" disabled state during the reload.
      setSaving(false);
      await load();
    } catch (err: unknown) {
      if (!mounted.current) return;
      setFormError(err instanceof Error ? err.message : String(err));
      setSaving(false);
    }
  }

  // entryId → key, so activity rows can name the key they touched. Only maps
  // CURRENTLY-listed entries — an activity row for an entry no longer in the
  // list resolves to "—" (M3.3 keeps entries, so this is rare).
  const keyByEntryId = useMemo(
    () => new Map(entries.map((e) => [e.id, e.key])),
    [entries],
  );

  const q = search.trim().toLowerCase();
  const visibleEntries = useMemo(
    () => (q ? entries.filter((e) => e.key.toLowerCase().includes(q)) : entries),
    [entries, q],
  );

  const wsLabel = workspaceName ?? "workspace";

  return (
    <div className="flex flex-1 min-w-0 overflow-hidden">
      {/* ── Center: blackboard table ─────────────────────────────────── */}
      <main className="flex-1 flex flex-col min-w-0 bg-surface">
        {/* Header */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2.5">
            <div className="w-6 h-6 rounded-[7px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
              <Layers className="w-[13px] h-[13px]" />
            </div>
            <div className="text-[13px] font-semibold tracking-tight flex items-center gap-1.5">
              Blackboard
              <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md">
                {wsLabel} · shared
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7 w-44">
              <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search keys"
                className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
              />
            </div>
            <button
              onClick={() => {
                setShowForm((v) => !v);
                setFormError(null);
              }}
              className="text-[12px] font-semibold text-white bg-accent px-3 h-7 rounded-lg hover:brightness-105 flex items-center gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" />
              New key
            </button>
            {onClose && (
              <button
                onClick={onClose}
                className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted shrink-0"
                aria-label="Close Blackboard"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        </div>

        {/* New-key inline form */}
        {showForm && (
          <div className="px-5 py-3 border-b border-overlay/[0.06] bg-sidebar shrink-0 space-y-2">
            <div className="flex items-center gap-2">
              <input
                value={newKey}
                onChange={(e) => setNewKey(e.target.value)}
                placeholder="key (e.g. plan/tasks)"
                className="flex-1 bg-surface ring-hair rounded-lg px-2.5 h-8 text-[12px] font-mono outline-none placeholder:text-text-tertiary focus:ring-accent/40"
              />
            </div>
            <textarea
              value={newValue}
              onChange={(e) => setNewValue(e.target.value)}
              placeholder='value — JSON or plain text (e.g. { "ci": "green" })'
              rows={3}
              className="w-full bg-surface ring-hair rounded-lg px-2.5 py-2 text-[12px] font-mono outline-none placeholder:text-text-tertiary focus:ring-accent/40 resize-y scroll-thin"
            />
            {formError && <p className="text-[11px] text-danger px-0.5">{formError}</p>}
            <div className="flex items-center gap-2">
              <button
                onClick={handleCreate}
                disabled={saving}
                className="text-[12px] font-semibold text-white bg-accent px-3 h-7 rounded-lg hover:brightness-105 disabled:opacity-40 flex items-center gap-1.5"
              >
                <Plus className="w-3.5 h-3.5" />
                {saving ? "Saving…" : "Save"}
              </button>
              <button
                onClick={() => {
                  setShowForm(false);
                  setFormError(null);
                }}
                className="text-[12px] font-medium text-text-secondary px-3 h-7 rounded-lg hover:bg-overlay/[0.05]"
              >
                Cancel
              </button>
              <span className="text-[10.5px] text-text-tertiary">
                User writes aren't attributed to an agent
              </span>
            </div>
          </div>
        )}

        {/* Table */}
        <div className="flex-1 overflow-y-auto scroll-thin">
          {/* Column head */}
          <div className="grid grid-cols-[1.4fr_2fr_1fr_0.8fr] gap-3 px-5 h-8 items-center text-[10px] font-bold tracking-wider text-text-tertiary uppercase border-b border-overlay/[0.06] sticky top-0 bg-surface/90 backdrop-blur z-10">
            <span>Key</span>
            <span>Value</span>
            <span>Last writer</span>
            <span>Updated</span>
          </div>

          {loading ? (
            <div className="grid place-items-center py-16 text-[13px] text-text-tertiary">
              Loading…
            </div>
          ) : visibleEntries.length === 0 ? (
            <div className="grid place-items-center py-16 text-[13px] text-text-tertiary px-6 text-center">
              {loadError
                ? "Failed to load the blackboard"
                : entries.length === 0
                  ? "No keys in this blackboard yet"
                  : "No keys match your search"}
            </div>
          ) : (
            <div className="divide-y divide-overlay/[0.05] text-[12.5px]">
              {visibleEntries.map((entry) => {
                const writer = entry.lastWriterId
                  ? agents.get(entry.lastWriterId)
                  : undefined;
                const isExpanded = expandedKey === entry.key;
                return (
                  <div key={entry.id}>
                    <button
                      onClick={() =>
                        setExpandedKey((k) => (k === entry.key ? null : entry.key))
                      }
                      className="w-full grid grid-cols-[1.4fr_2fr_1fr_0.8fr] gap-3 px-5 py-2.5 items-center text-left hover:bg-overlay/[0.02]"
                    >
                      <span className="font-mono text-[12px] text-text-body truncate flex items-center gap-1.5">
                        {isExpanded ? (
                          <ChevronDown className="w-3 h-3 text-text-tertiary shrink-0" />
                        ) : (
                          <ChevronRight className="w-3 h-3 text-text-tertiary shrink-0" />
                        )}
                        {entry.key}
                      </span>
                      <span className="text-text-secondary truncate">
                        {previewValue(entry.value)}
                      </span>
                      <span className="flex items-center gap-1.5 min-w-0">
                        {writer ? (
                          <>
                            <WriterAvatar identity={writer} />
                            <span className="text-text-body truncate">{writer.name}</span>
                          </>
                        ) : (
                          <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.05] px-1.5 py-px rounded">
                            You
                          </span>
                        )}
                      </span>
                      <span className="text-text-muted text-[11.5px]">
                        {timeHint(entry.updatedAt)}
                      </span>
                    </button>
                    {isExpanded && (
                      <div className="px-5 pb-3 pt-0.5">
                        <pre className="font-mono text-[11px] whitespace-pre-wrap break-words rounded-lg ring-hair bg-sidebar p-2.5 text-text-body">
                          {prettyValue(entry.value)}
                        </pre>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </main>

      {/* ── Right: activity / access ─────────────────────────────────── */}
      <aside className="w-[306px] vibrancy border-l border-overlay/[0.06] flex flex-col shrink-0">
        <div className="h-12 flex items-center px-4 border-b border-overlay/[0.06] shrink-0">
          <span className="text-[12px] font-semibold text-text-secondary tracking-tight">
            Activity
          </span>
        </div>
        <div className="flex-1 overflow-y-auto scroll-thin p-3 space-y-4">
          {/* Recent writes / reads — REAL activity, newest first. */}
          <div>
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
              Recent writes
            </div>
            {activity.length === 0 ? (
              <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 text-[11.5px] text-text-tertiary">
                No activity yet
              </div>
            ) : (
              <div className="space-y-2">
                {activity.map((act) => {
                  const who = agents.get(act.instanceId);
                  const key = keyByEntryId.get(act.entryId);
                  const verb =
                    act.action === "write" ? "wrote" : act.action === "read" ? "read" : act.action;
                  return (
                    <div key={act.id} className="flex items-start gap-2 text-[11.5px]">
                      <span
                        className="w-4 h-4 rounded-[5px] text-white grid place-items-center text-[9px] font-bold mt-0.5 shrink-0"
                        style={{ backgroundColor: who?.color ?? "#86868b" }}
                      >
                        {(who?.name ?? "?").charAt(0).toUpperCase()}
                      </span>
                      <div className="min-w-0">
                        <span
                          className="font-semibold"
                          style={{ color: who?.color ?? "#3a3a3c" }}
                        >
                          {who?.name ?? act.instanceId}
                        </span>{" "}
                        <span className="text-text-secondary">{verb}</span>{" "}
                        <span className="font-mono text-[11px]">{key ?? "—"}</span>
                        <div className="text-[10px] text-text-muted">{timeHint(act.at)}</div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Access — STATIC but HONEST (M3.3 model: any agent in the ws can
              read/write; cross-workspace is blocked). No per-key ACLs exist. */}
          <div>
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
              Access
            </div>
            <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-2 text-[11.5px]">
              <div className="flex items-center justify-between">
                <span className="text-text-secondary flex items-center gap-1.5">
                  <Eye className="w-3.5 h-3.5" />
                  Readers
                </span>
                <span className="font-medium">All agents</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-text-secondary flex items-center gap-1.5">
                  <Pencil className="w-3.5 h-3.5" />
                  Writers
                </span>
                <span className="font-medium">All agents</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-text-secondary flex items-center gap-1.5">
                  <Lock className="w-3.5 h-3.5" />
                  Scope
                </span>
                <span className="font-medium">This workspace</span>
              </div>
            </div>
            <div className="mt-1.5 text-[10.5px] text-text-muted leading-snug px-0.5">
              This blackboard is shared only within the{" "}
              <span className="font-medium text-text-body">{wsLabel}</span> workspace — other workspaces have their own.
            </div>
          </div>
        </div>
      </aside>
    </div>
  );
}
