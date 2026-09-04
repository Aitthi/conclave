import { useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  CirclePause,
  Folder,
  History,
  LoaderCircle,
  MessageSquare,
  MoreHorizontal,
  Play,
  Search,
  Settings2,
  Shield,
  Square,
  Terminal,
  Users,
  Waypoints,
  X,
} from "lucide-react";

export const meta = { title: "Agent lifecycle — stop and resume" };

type PreviewState = "stopped" | "resuming" | "resume-error" | "active";
type WorkspacePreview = "started" | "stopped" | "starting" | "partial";

const identity = {
  aoki: "var(--color-agent-indigo)",
  hardwell: "var(--color-agent-magenta)",
  dew: "var(--color-agent-teal)",
};

function Avatar({ name, color }: { name: string; color: string }) {
  return (
    <span
      className="grid h-7 w-7 shrink-0 place-items-center rounded-[8px] text-[12px] font-bold text-white"
      style={{ backgroundColor: color }}
      aria-hidden="true"
    >
      {name[0]}
    </span>
  );
}

function RailIcon({ active, children, label }: { active?: boolean; children: React.ReactNode; label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      className={`grid h-10 w-10 place-items-center rounded-[10px] transition-colors ${
        active ? "bg-accent text-white" : "text-text-secondary hover:bg-overlay/[0.06] hover:text-text-primary"
      }`}
    >
      {children}
    </button>
  );
}

function StatusDot({ state }: { state: "running" | "idle" | "stopped" }) {
  if (state === "stopped") {
    return <CirclePause className="h-3.5 w-3.5 shrink-0 text-text-tertiary" aria-label="Stopped" />;
  }
  return (
    <span
      className={`h-2 w-2 shrink-0 rounded-full ${state === "running" ? "bg-live" : "bg-text-tertiary"}`}
      role="img"
      aria-label={state}
    />
  );
}

function AgentRow({
  name,
  role,
  color,
  state,
  working,
  selected,
  action,
  onAction,
  availabilityLabel,
  dimmed,
}: {
  name: string;
  role: string;
  color: string;
  state: "running" | "idle" | "stopped";
  working?: boolean;
  selected?: boolean;
  action?: "stop" | "resume";
  onAction?: () => void;
  availabilityLabel?: string;
  dimmed?: boolean;
}) {
  return (
    <div
      className={`group flex min-h-11 w-full items-start gap-2.5 rounded-lg px-2 py-1.5 transition-colors ${
        selected ? "bg-accent/[0.08] ring-1 ring-accent/20" : "hover:bg-overlay/[0.04]"
      } ${dimmed ? "opacity-70" : ""}`}
    >
      <Avatar name={name} color={color} />
      <div className="min-w-0 flex-1 leading-tight">
        <div className="flex items-center gap-1.5">
          <span className={`truncate text-[12.5px] font-semibold ${state === "stopped" ? "text-text-secondary" : "text-text-primary"}`}>
            {name}
          </span>
          <Terminal className="h-3 w-3 shrink-0 text-text-muted" />
        </div>
        <div className="mt-0.5 flex items-center gap-1 text-[10.5px] text-text-muted">
          <span>{role}</span>
          {state === "stopped" && (
            <span className="inline-flex items-center gap-1 rounded-md bg-fill px-1.5 py-0.5 font-semibold text-text-secondary">
              <CirclePause className="h-2.5 w-2.5" />
              Stopped
            </span>
          )}
        </div>
        {state !== "stopped" && availabilityLabel && (
          <div className="mt-0.5 inline-flex rounded-md bg-fill px-1.5 py-0.5 text-[9.5px] font-semibold text-text-secondary">
            {availabilityLabel}
          </div>
        )}
        {working && (
          <div className="mt-0.5 flex items-center gap-1 text-[10px] font-semibold text-waiting">
            <LoaderCircle className="h-2.5 w-2.5 animate-spin motion-reduce:animate-none" />
            working…
          </div>
        )}
      </div>
      {action && (
        <button
          type="button"
          onClick={onAction}
          aria-label={`${action === "stop" ? "Stop" : "Resume"} agent ${name}`}
          className={`mt-0.5 inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-[10.5px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
            action === "resume"
              ? "bg-accent text-white hover:bg-accent-hover"
              : "bg-overlay/[0.06] text-text-secondary hover:bg-overlay/[0.10] hover:text-text-primary"
          }`}
        >
          {action === "resume" ? <Play className="h-3 w-3" /> : <Square className="h-2.5 w-2.5" />}
          {action === "resume" ? "Resume" : "Stop"}
        </button>
      )}
      <button
        type="button"
        aria-label={`More actions for ${name}`}
        className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md text-text-muted transition-colors hover:bg-overlay/[0.06] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      >
        <MoreHorizontal className="h-3.5 w-3.5" />
      </button>
      <StatusDot state={state} />
    </div>
  );
}

function ChatRail({ routeOpen, onToggleRoute }: { routeOpen: boolean; onToggleRoute: () => void }) {
  return (
    <aside className="flex w-[238px] shrink-0 flex-col border-l border-border bg-sidebar">
      <div className="flex h-12 items-center border-b border-border px-3 text-[13px] font-semibold">Chats</div>
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <span className="rounded-full bg-accent/[0.14] px-2.5 py-1 text-[11.5px] font-semibold text-accent ring-1 ring-accent/30">
          # workspace
        </span>
        <span className="truncate rounded-full bg-fill px-2.5 py-1 text-[11.5px] text-text-secondary">Aoki · Dew</span>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-hidden px-3 py-4">
        <div>
          <div className="flex items-center gap-2 text-[11px] text-text-muted">
            <Avatar name="Aoki" color={identity.aoki} />
            <span className="font-semibold text-text-secondary">Aoki</span>
            <span>13:02</span>
          </div>
          <p className="ml-9 mt-1.5 rounded-[10px] bg-fill px-3 py-2.5 text-[12px] leading-relaxed text-text-secondary">
            Hardwell is stopped. Route new work to an active agent or resume that agent first.
          </p>
        </div>
        <div>
          <div className="flex items-center gap-2 text-[11px] text-text-muted">
            <Avatar name="Dew" color={identity.dew} />
            <span className="font-semibold text-text-secondary">Dew</span>
            <span>13:04</span>
          </div>
          <p className="ml-9 mt-1.5 rounded-[10px] bg-fill px-3 py-2.5 text-[12px] leading-relaxed text-text-secondary">
            The existing conversation remains readable while the runtime is stopped.
          </p>
        </div>
      </div>
      <div className="relative border-t border-border p-2.5">
        {routeOpen && <RoutingSpecimen />}
        <button
          type="button"
          onClick={onToggleRoute}
          aria-expanded={routeOpen}
          className="flex h-8 w-full items-center gap-2 rounded-lg bg-fill px-2.5 text-left text-[11px] text-text-secondary ring-1 ring-border transition-colors hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <MessageSquare className="h-3.5 w-3.5 text-accent" />
          <span className="min-w-0 flex-1 truncate"><span className="font-semibold text-text-primary">Send to Aoki</span> · self</span>
          <ChevronDown className={`h-3 w-3 transition-transform ${routeOpen ? "rotate-180" : ""}`} />
        </button>
        <p className="mt-1.5 px-1 text-[9.5px] leading-snug text-text-tertiary">
          Stopped agents cannot receive new messages
        </p>
      </div>
    </aside>
  );
}

function StoppedPane({ state, setState }: { state: PreviewState; setState: (state: PreviewState) => void }) {
  const resuming = state === "resuming";
  const failed = state === "resume-error";

  if (state === "active") {
    return (
      <div className="flex min-h-0 flex-1 flex-col bg-canvas">
        <div className="flex-1 px-5 py-4 font-mono text-[12px] leading-relaxed text-text-secondary">
          <p className="text-live">runtime ready</p>
          <p>Hardwell resumed as the same workspace agent.</p>
          <span className="mt-3 inline-block h-4 w-1.5 bg-text-primary" aria-hidden="true" />
        </div>
        <div className="border-t border-white/[0.08] bg-surface px-4 py-3">
          <div className="rounded-[12px] bg-fill px-3 py-2 text-[12px] text-text-muted">Message Hardwell…</div>
        </div>
      </div>
    );
  }

  return (
    <div className="grid min-h-0 flex-1 place-items-center bg-surface px-6">
      <div className="flex max-w-md flex-col items-center text-center">
        <div className="grid h-12 w-12 place-items-center rounded-[12px] bg-fill text-text-secondary">
          {resuming ? (
            <LoaderCircle className="h-6 w-6 animate-spin text-accent motion-reduce:animate-none" />
          ) : (
            <CirclePause className="h-6 w-6" />
          )}
        </div>
        <h1 className="mt-4 text-[18px] font-semibold tracking-[-0.02em] text-text-primary">
          {resuming ? "Resuming Hardwell…" : "Hardwell is stopped"}
        </h1>
        <p className="mt-2 max-w-[46ch] text-[12.5px] leading-relaxed text-text-secondary">
          {resuming
            ? "Starting a fresh runtime for the same workspace agent."
            : "No agent runtime is running. Hardwell remains in codeup with its role, configuration, supervisor links, and history."}
        </p>
        <button
          type="button"
          disabled={resuming}
          onClick={() => setState("resuming")}
          className="mt-5 inline-flex min-w-32 items-center justify-center gap-1.5 rounded-md bg-accent px-3.5 py-2 text-[12.5px] font-semibold text-white transition-colors hover:bg-accent-hover disabled:cursor-wait disabled:opacity-65 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface"
        >
          {resuming ? <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Play className="h-3.5 w-3.5" />}
          {resuming ? "Resuming agent…" : "Resume agent"}
        </button>
        {!resuming && (
          <p className="mt-2 text-[10.5px] text-text-tertiary">Starts a fresh runtime; saved records stay attached.</p>
        )}
        {failed && (
          <div role="alert" className="mt-3 flex max-w-sm items-start gap-2 rounded-lg bg-danger/[0.10] px-3 py-2 text-left text-[11.5px] leading-relaxed text-danger">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>Couldn’t resume Hardwell. The agent remains stopped. Check the CLI setup, then try again.</span>
          </div>
        )}
      </div>
    </div>
  );
}

function WorkspaceStatusPane({ state, setState }: { state: WorkspacePreview; setState: (state: WorkspacePreview) => void }) {
  const starting = state === "starting";
  const partial = state === "partial";

  return (
    <div className="grid min-h-0 flex-1 place-items-center bg-surface px-6">
      <div className="flex max-w-lg flex-col items-center text-center">
        <div className={`grid h-12 w-12 place-items-center rounded-[12px] bg-fill ${partial ? "text-waiting" : starting ? "text-accent" : "text-text-secondary"}`}>
          {starting ? (
            <LoaderCircle className="h-6 w-6 animate-spin motion-reduce:animate-none" />
          ) : partial ? (
            <AlertCircle className="h-6 w-6" />
          ) : (
            <CirclePause className="h-6 w-6" />
          )}
        </div>
        <h1 className="mt-4 text-[18px] font-semibold tracking-[-0.02em] text-text-primary">
          {starting ? "Starting codeup…" : partial ? "codeup started with an issue" : "codeup is stopped"}
        </h1>
        <p className="mt-2 max-w-[52ch] text-[12.5px] leading-relaxed text-text-secondary">
          {starting
            ? "Launching Aoki and Dew. Hardwell stays stopped because its individual availability is stopped."
            : partial
              ? "Aoki started. Dew could not start, and Hardwell stayed stopped by design. Retained workspace records remain available."
              : "Inspect agents, tasks, messages, configuration, and history without launching any agent runtime."}
        </p>

        {starting && (
          <div className="mt-4 w-full max-w-xs space-y-1.5 text-left text-[11.5px]">
            <div className="flex items-center gap-2 rounded-md bg-fill px-2.5 py-1.5"><LoaderCircle className="h-3 w-3 animate-spin text-accent motion-reduce:animate-none" /><span className="flex-1">Aoki</span><span className="text-text-muted">Starting…</span></div>
            <div className="flex items-center gap-2 rounded-md bg-fill px-2.5 py-1.5"><CirclePause className="h-3 w-3 text-text-tertiary" /><span className="flex-1">Hardwell</span><span className="text-text-muted">Individually stopped</span></div>
            <div className="flex items-center gap-2 rounded-md bg-fill px-2.5 py-1.5"><LoaderCircle className="h-3 w-3 animate-spin text-accent motion-reduce:animate-none" /><span className="flex-1">Dew</span><span className="text-text-muted">Queued</span></div>
          </div>
        )}

        {partial && (
          <div role="alert" className="mt-4 flex max-w-sm items-start gap-2 rounded-lg bg-waiting/[0.10] px-3 py-2 text-left text-[11.5px] leading-relaxed text-waiting">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>Couldn’t start Dew: CLI executable is unavailable. The workspace remains started with Aoki active.</span>
          </div>
        )}

        <div className="mt-5 flex items-center gap-2">
          <button
            type="button"
            disabled={starting}
            onClick={() => setState("starting")}
            className="inline-flex min-w-32 items-center justify-center gap-1.5 rounded-md bg-accent px-3.5 py-2 text-[12.5px] font-semibold text-white transition-colors hover:bg-accent-hover disabled:cursor-wait disabled:opacity-65 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface"
          >
            {starting ? <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Play className="h-3.5 w-3.5" />}
            {starting ? "Starting workspace…" : partial ? "Retry 1 agent" : "Start workspace"}
          </button>
          {partial && (
            <button type="button" onClick={() => setState("stopped")} className="rounded-md bg-overlay/[0.06] px-3 py-2 text-[12px] font-semibold text-text-secondary hover:bg-overlay/[0.10] hover:text-text-primary">
              Stop workspace
            </button>
          )}
        </div>
        {!starting && !partial && (
          <p className="mt-2 text-[10.5px] text-text-tertiary">Starts 2 individually active agents · keeps 1 individually stopped</p>
        )}
      </div>
    </div>
  );
}

function WorkspaceFrame({ preview, setPreview, workspacePreview, setWorkspacePreview, openStop, openRemove, openWorkspaceStop, routeOpen, onToggleRoute }: {
  preview: PreviewState;
  setPreview: (state: PreviewState) => void;
  workspacePreview: WorkspacePreview;
  setWorkspacePreview: (state: WorkspacePreview) => void;
  openStop: () => void;
  openRemove: () => void;
  openWorkspaceStop: () => void;
  routeOpen: boolean;
  onToggleRoute: () => void;
}) {
  const workspaceStarted = workspacePreview === "started";
  const workspaceRunning = workspacePreview === "started" || workspacePreview === "partial";
  const stopped = preview !== "active";
  return (
    <div className="flex h-full min-h-0 w-full overflow-hidden rounded-[14px] bg-surface ring-1 ring-border">
      <nav className="flex w-[54px] shrink-0 flex-col items-center gap-2 border-r border-border bg-canvas px-1.5 py-3">
        <RailIcon label="Workspaces"><Waypoints className="h-4 w-4" /></RailIcon>
        <RailIcon active label="codeup workspace"><span className="text-[12px] font-bold">C</span></RailIcon>
        <RailIcon label="Library"><Users className="h-4 w-4" /></RailIcon>
        <RailIcon label="Chat"><MessageSquare className="h-4 w-4" /></RailIcon>
        <div className="flex-1" />
        <RailIcon label="Settings"><Settings2 className="h-4 w-4" /></RailIcon>
      </nav>

      <aside className="flex w-[248px] shrink-0 flex-col border-r border-border bg-sidebar">
        <div className="flex h-12 items-center gap-2 border-b border-border px-3.5">
          <span className="grid h-6 w-6 place-items-center rounded-[7px] bg-accent text-[11px] font-bold text-white">C</span>
          <div className="min-w-0 flex-1 leading-tight">
            <div className="flex items-center gap-1.5 truncate text-[12.5px] font-semibold">
              <span>codeup</span>
              {!workspaceRunning && (
                <span className="inline-flex items-center gap-1 rounded-md bg-fill px-1.5 py-0.5 text-[9.5px] font-semibold text-text-secondary">
                  <CirclePause className="h-2.5 w-2.5" /> {workspacePreview === "starting" ? "Starting" : "Stopped"}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1 truncate font-mono text-[9.5px] text-text-muted">
              <Folder className="h-2.5 w-2.5" /> /Users/dev/code/codeup
            </div>
          </div>
          <button
            type="button"
            disabled={workspacePreview === "starting"}
            onClick={workspaceRunning ? openWorkspaceStop : () => setWorkspacePreview("starting")}
            aria-label={workspaceRunning ? "Stop workspace codeup" : "Start workspace codeup"}
            className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md bg-overlay/[0.06] px-1.5 text-[10px] font-semibold text-text-secondary hover:bg-overlay/[0.10] hover:text-text-primary disabled:opacity-50"
          >
            {workspacePreview === "starting" ? <LoaderCircle className="h-3 w-3 animate-spin motion-reduce:animate-none" /> : workspaceRunning ? <Square className="h-2.5 w-2.5" /> : <Play className="h-3 w-3" />}
            {workspacePreview === "starting" ? "Starting" : workspaceRunning ? "Stop" : "Start"}
          </button>
        </div>
        <div className="px-3 pb-2 pt-3">
          <div className="flex h-7 items-center gap-2 rounded-lg bg-overlay/[0.05] px-2.5">
            <Search className="h-3.5 w-3.5 text-text-muted" />
            <span className="text-[12px] text-text-tertiary">Search agents</span>
          </div>
        </div>
        <div className="min-h-0 flex-1 px-2 pb-2">
          <div className="px-2 pb-1 text-[10px] font-bold uppercase tracking-[0.08em] text-text-tertiary">CLI agents</div>
          <div className="space-y-0.5">
            <AgentRow
              name="Aoki"
              role="Lead"
              color={identity.aoki}
              state={workspaceRunning ? "running" : "idle"}
              working={workspaceStarted}
              action={workspaceStarted ? "stop" : undefined}
              onAction={workspaceStarted ? openStop : undefined}
              availabilityLabel={!workspaceRunning ? (workspacePreview === "starting" ? "Starting" : "Auto-start") : undefined}
              dimmed={!workspaceRunning}
            />
            <AgentRow
              name="Hardwell"
              role="Designer"
              color={identity.hardwell}
              state={stopped ? "stopped" : "running"}
              selected={workspaceStarted}
              dimmed={!workspaceRunning}
            />
            <AgentRow
              name="Dew"
              role="Implementer"
              color={identity.dew}
              state="idle"
              action={workspaceStarted ? "stop" : undefined}
              onAction={workspaceStarted ? openStop : undefined}
              availabilityLabel={workspacePreview === "partial" ? "Start failed" : !workspaceRunning ? (workspacePreview === "starting" ? "Queued" : "Auto-start") : undefined}
              dimmed={!workspaceRunning || workspacePreview === "partial"}
            />
          </div>
          <p className="px-2 pt-3 text-[10.5px] leading-relaxed text-text-tertiary">
            Remove lives in More actions and always uses a separate destructive confirmation.
          </p>
          <button
            type="button"
            onClick={openRemove}
            className="mx-2 mt-2 inline-flex items-center gap-1 text-[10.5px] font-semibold text-danger hover:underline"
          >
            Preview Remove confirmation
          </button>
        </div>
        <div className="border-t border-border p-2 text-[12px] text-text-secondary">
          <div className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-overlay/[0.04]">
            <MessageSquare className="h-4 w-4" />
            <span className="font-semibold">Chat</span>
          </div>
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col bg-surface">
        <div className="flex h-12 items-center gap-1 border-b border-border bg-sidebar px-2">
          {workspaceStarted ? (
            <button type="button" className="flex h-7 items-center gap-2 rounded-md bg-overlay/[0.06] px-3 text-[12.5px] font-semibold">
              <StatusDot state={stopped ? "stopped" : "running"} />
              <span>Hardwell</span>
              {stopped && <span className="text-[10px] font-medium text-text-muted">Stopped</span>}
              <Terminal className="h-3 w-3 text-text-muted" />
            </button>
          ) : (
            <div className="flex h-7 items-center gap-2 rounded-md bg-overlay/[0.06] px-3 text-[12.5px] font-semibold">
              {workspacePreview === "partial" ? <AlertCircle className="h-3.5 w-3.5 text-waiting" /> : <CirclePause className="h-3.5 w-3.5 text-text-tertiary" />}
              <span>Workspace</span>
              <span className="text-[10px] font-medium text-text-muted">{workspacePreview === "starting" ? "Starting" : workspacePreview === "partial" ? "Started with issue" : "Stopped"}</span>
            </div>
          )}
        </div>
        {workspaceStarted ? (
          <div className="flex h-10 items-center gap-2 border-b border-border px-3 text-[11.5px] text-text-secondary">
            <button type="button" className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-overlay/[0.05]">
              <Shield className="h-3.5 w-3.5 text-accent" /> Skills <ChevronDown className="h-3 w-3" />
            </button>
            <span className="h-4 w-px bg-border" />
            <button type="button" className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-overlay/[0.05]" title="Restore a saved snapshot">
              <History className="h-3.5 w-3.5" /> Restore snapshot
            </button>
            <div className="flex-1" />
            <button type="button" className="rounded-md p-1.5 hover:bg-overlay/[0.05]" aria-label="Agent configuration">
              <Settings2 className="h-3.5 w-3.5" />
            </button>
          </div>
        ) : (
          <div className="flex h-10 items-center gap-2 border-b border-border px-4 text-[11.5px] text-text-muted">
            {workspacePreview === "partial" ? (
              <>
                <AlertCircle className="h-3.5 w-3.5 text-waiting" />
                <span>1 agent active · 1 failed · 1 individually stopped</span>
              </>
            ) : workspacePreview === "starting" ? (
              <>
                <LoaderCircle className="h-3.5 w-3.5 animate-spin text-accent motion-reduce:animate-none" />
                <span>Starting individually active agents · stopped agents are skipped</span>
              </>
            ) : (
              <>
                <CirclePause className="h-3.5 w-3.5" />
                <span>Inspect only · retained workspace records are available</span>
              </>
            )}
          </div>
        )}
        {workspaceStarted ? (
          <StoppedPane state={preview} setState={setPreview} />
        ) : (
          <WorkspaceStatusPane state={workspacePreview} setState={setWorkspacePreview} />
        )}
      </main>

      <ChatRail routeOpen={routeOpen} onToggleRoute={onToggleRoute} />
    </div>
  );
}

function RoutingSpecimen() {
  return (
    <div className="absolute bottom-[62px] left-2.5 right-2.5 z-20 rounded-[12px] bg-surface-raised p-1.5 shadow-xl">
      <div className="px-2 pb-1.5 pt-1 text-[10px] font-semibold text-text-tertiary">Send to</div>
      <div className="w-full">
        <button type="button" className="flex w-full items-center gap-2 rounded-lg bg-accent/[0.10] px-2 py-2 text-left text-[12px]">
          <Avatar name="Aoki" color={identity.aoki} />
          <span className="min-w-0 flex-1 font-medium">Aoki <span className="font-normal text-text-tertiary">· self</span></span>
          <Check className="h-3.5 w-3.5 text-accent" />
        </button>
        <button
          type="button"
          disabled
          aria-disabled="true"
          aria-label="Hardwell, stopped and unavailable"
          className="flex w-full cursor-not-allowed items-center gap-2 rounded-lg px-2 py-2 text-left opacity-55"
        >
          <Avatar name="Hardwell" color={identity.hardwell} />
          <span className="min-w-0 flex-1 text-[12px] font-medium text-text-secondary">Hardwell</span>
          <span className="inline-flex items-center gap-1 text-[10.5px] font-semibold text-text-tertiary"><CirclePause className="h-3 w-3" /> Stopped</span>
        </button>
        <button type="button" className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-[12px] hover:bg-overlay/[0.04]">
          <Avatar name="Dew" color={identity.dew} />
          <span className="min-w-0 flex-1 font-medium">Dew</span>
          <span className="text-[10.5px] text-text-tertiary">stdin</span>
        </button>
      </div>
    </div>
  );
}

function ConfirmDialog({ kind, close, confirm }: { kind: "stop" | "remove" | "workspace-stop"; close: () => void; confirm: () => void }) {
  const remove = kind === "remove";
  const workspaceStop = kind === "workspace-stop";
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/55 px-5" role="presentation">
      <div role="dialog" aria-modal="true" aria-labelledby="confirm-title" className="w-full max-w-[380px] rounded-[14px] bg-surface-raised p-5 shadow-xl">
        <div className="flex items-start gap-3">
          <div className={`grid h-9 w-9 shrink-0 place-items-center rounded-[10px] ${remove ? "bg-danger/[0.12] text-danger" : "bg-waiting/[0.12] text-waiting"}`}>
            {remove ? <X className="h-4.5 w-4.5" /> : <Square className="h-4 w-4" />}
          </div>
          <div>
            <h2 id="confirm-title" className="text-[14px] font-semibold text-text-primary">
              {remove ? "Remove Hardwell from codeup?" : workspaceStop ? "Stop codeup while an agent is working?" : "Stop Aoki while working?"}
            </h2>
            <p className="mt-1.5 text-[12px] leading-relaxed text-text-secondary">
              {remove
                ? "This removes workspace membership and its attached workspace records. This is separate from stopping the runtime."
                : workspaceStop
                  ? "All live agent runtimes and current work terminate immediately. The workspace, agents, configuration, tasks, messages, and history stay."
                : "The current runtime and work terminate immediately. Workspace membership, configuration, supervisor links, and history stay."}
            </p>
          </div>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={close} className="rounded-md bg-fill px-3 py-1.5 text-[12px] font-semibold text-text-secondary hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">Cancel</button>
          <button type="button" autoFocus onClick={confirm} className={`rounded-md px-3 py-1.5 text-[12px] font-semibold text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${remove ? "bg-danger" : "bg-text-primary text-surface"}`}>
            {remove ? "Remove agent" : workspaceStop ? "Stop workspace" : "Stop agent"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function AgentStopResume() {
  const [preview, setPreview] = useState<PreviewState>("stopped");
  const [workspacePreview, setWorkspacePreview] = useState<WorkspacePreview>("started");
  const [dialog, setDialog] = useState<"stop" | "remove" | "workspace-stop" | null>(null);
  const [routeOpen, setRouteOpen] = useState(false);
  const workspaceStarted = workspacePreview === "started";

  return (
    <div className="dark h-screen overflow-hidden bg-canvas font-sans text-text-primary antialiased">
      <div className="flex h-full min-h-0 flex-col gap-3 p-3">
        <header className="flex min-h-[64px] shrink-0 items-center gap-5 rounded-[12px] bg-sidebar px-4 ring-1 ring-border">
          <div className="flex min-w-0 items-center gap-3">
            <div className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-accent/[0.14] text-accent ring-1 ring-accent/25">
              <Waypoints className="h-[18px] w-[18px]" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="truncate text-[15px] font-semibold tracking-[-0.015em]">Lifecycle controls</h1>
                <span className="rounded-full bg-fill px-2 py-0.5 text-[9.5px] font-semibold text-text-muted">Home canon</span>
              </div>
              <p className="mt-0.5 truncate text-[10.5px] text-text-muted">Stop runtimes, keep identity and records</p>
            </div>
          </div>

          <div className="ml-auto flex min-w-0 items-center gap-3 overflow-x-auto" aria-label="Canon state controls">
            <div className="flex shrink-0 items-center gap-1.5" aria-label="Workspace preview state">
              <span className="text-[9.5px] font-semibold text-text-tertiary">Workspace</span>
              <div className="flex items-center rounded-lg bg-canvas p-1">
                {([
                  ["started", "Started"],
                  ["stopped", "Stopped"],
                  ["starting", "Starting"],
                  ["partial", "Partial"],
                ] as const).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    onClick={() => setWorkspacePreview(value)}
                    aria-pressed={workspacePreview === value}
                    className={`rounded-md px-2 py-1 text-[10px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                      workspacePreview === value ? "bg-surface-raised text-text-primary" : "text-text-muted hover:text-text-primary"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>

            <span className="h-6 w-px shrink-0 bg-border" />

            <div className="flex shrink-0 items-center gap-1.5" aria-label="Agent preview state">
              <span className="text-[9.5px] font-semibold text-text-tertiary">Agent</span>
              <div className="flex items-center rounded-lg bg-canvas p-1">
                {([
                  ["stopped", "Stopped"],
                  ["resuming", "Resuming"],
                  ["resume-error", "Error"],
                  ["active", "Resumed"],
                ] as const).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    disabled={!workspaceStarted}
                    onClick={() => setPreview(value)}
                    aria-pressed={preview === value}
                    className={`rounded-md px-2 py-1 text-[10px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                      preview === value ? "bg-surface-raised text-text-primary" : "text-text-muted hover:text-text-primary"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </header>

        <div className="min-h-0 flex-1">
          <WorkspaceFrame
            preview={preview}
            setPreview={setPreview}
            workspacePreview={workspacePreview}
            setWorkspacePreview={setWorkspacePreview}
            openStop={() => setDialog("stop")}
            openRemove={() => setDialog("remove")}
            openWorkspaceStop={() => setDialog("workspace-stop")}
            routeOpen={routeOpen}
            onToggleRoute={() => setRouteOpen((open) => !open)}
          />
        </div>

        <footer className="flex h-8 shrink-0 items-center gap-4 px-1 text-[9.5px] text-text-tertiary">
          <span className="inline-flex items-center gap-1.5"><span className="h-1.5 w-1.5 rounded-full bg-live" /> Workspace: {workspacePreview}</span>
          <span className="inline-flex items-center gap-1.5"><CirclePause className="h-3 w-3" /> Agent: {preview === "resume-error" ? "resume error" : preview}</span>
          <span className="ml-auto">Implementation checks: home/default · home/empty</span>
        </footer>
      </div>

      {dialog && (
        <ConfirmDialog
          kind={dialog}
          close={() => setDialog(null)}
          confirm={() => {
            setDialog(null);
            if (dialog === "stop") setPreview("stopped");
            if (dialog === "workspace-stop") setWorkspacePreview("stopped");
          }}
        />
      )}
    </div>
  );
}
