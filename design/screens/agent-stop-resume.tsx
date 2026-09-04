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
          {state !== "stopped" && availabilityLabel && (
            <span className="truncate rounded-md bg-fill px-1.5 py-0.5 font-semibold text-text-secondary">
              {availabilityLabel}
            </span>
          )}
        </div>
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

function ChatRail() {
  return (
    <aside className="hidden w-[286px] shrink-0 border-l border-border bg-sidebar xl:flex xl:flex-col">
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
      <div className="border-t border-border px-3 py-2.5 text-[10.5px] text-text-tertiary">
        Read-only live view · stopped agents do not receive messages
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

function WorkspaceFrame({ preview, setPreview, workspacePreview, setWorkspacePreview, openStop, openRemove, openWorkspaceStop }: {
  preview: PreviewState;
  setPreview: (state: PreviewState) => void;
  workspacePreview: WorkspacePreview;
  setWorkspacePreview: (state: WorkspacePreview) => void;
  openStop: () => void;
  openRemove: () => void;
  openWorkspaceStop: () => void;
}) {
  const workspaceStarted = workspacePreview === "started";
  const workspaceRunning = workspacePreview === "started" || workspacePreview === "partial";
  const stopped = preview !== "active";
  return (
    <div className="flex h-[650px] min-w-[1040px] overflow-hidden rounded-[14px] bg-surface ring-1 ring-border">
      <nav className="flex w-[54px] shrink-0 flex-col items-center gap-2 border-r border-border bg-canvas px-1.5 py-3">
        <RailIcon label="Workspaces"><Waypoints className="h-4 w-4" /></RailIcon>
        <RailIcon active label="codeup workspace"><span className="text-[12px] font-bold">C</span></RailIcon>
        <RailIcon label="Library"><Users className="h-4 w-4" /></RailIcon>
        <RailIcon label="Chat"><MessageSquare className="h-4 w-4" /></RailIcon>
        <div className="flex-1" />
        <RailIcon label="Settings"><Settings2 className="h-4 w-4" /></RailIcon>
      </nav>

      <aside className="flex w-[266px] shrink-0 flex-col border-r border-border bg-sidebar">
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
            <CirclePause className="h-3.5 w-3.5" />
            <span>Inspect only · retained workspace records are available</span>
          </div>
        )}
        {workspaceStarted ? (
          <StoppedPane state={preview} setState={setPreview} />
        ) : (
          <WorkspaceStatusPane state={workspacePreview} setState={setWorkspacePreview} />
        )}
      </main>

      <ChatRail />
    </div>
  );
}

function RoutingSpecimen() {
  return (
    <div className="relative min-h-64 bg-surface px-5 py-4">
      <div className="text-[12px] font-semibold text-text-primary">Send to</div>
      <p className="mt-1 text-[11.5px] leading-relaxed text-text-secondary">Stopped agents remain visible for context but cannot be selected.</p>
      <div className="mt-3 w-full max-w-[300px] rounded-[12px] bg-surface-raised p-1 ring-1 ring-border">
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

function StateSpecimen({
  title,
  body,
  state,
  action,
  busy,
  danger,
}: {
  title: string;
  body: string;
  state: "running" | "stopped";
  action: string;
  busy?: boolean;
  danger?: boolean;
}) {
  return (
    <div className="min-w-[220px] flex-1 px-4 py-4">
      <div className="flex items-center gap-2">
        <StatusDot state={state} />
        <h3 className="text-[12.5px] font-semibold text-text-primary">{title}</h3>
      </div>
      <p className="mt-2 min-h-12 text-[11.5px] leading-relaxed text-text-secondary">{body}</p>
      <button
        type="button"
        disabled={busy}
        className={`mt-3 inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[11.5px] font-semibold ${
          danger ? "bg-danger text-white" : state === "stopped" ? "bg-accent text-white" : "bg-overlay/[0.06] text-text-secondary"
        } disabled:opacity-60`}
      >
        {busy ? <LoaderCircle className="h-3 w-3 animate-spin motion-reduce:animate-none" /> : state === "stopped" ? <Play className="h-3 w-3" /> : <Square className="h-2.5 w-2.5" />}
        {action}
      </button>
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
  const workspaceStarted = workspacePreview === "started";

  return (
    <div className="h-screen overflow-y-auto bg-canvas font-sans text-text-primary antialiased">
      <div className="mx-auto min-w-[1080px] max-w-[1540px] px-8 py-7">
        <header className="flex items-end justify-between gap-6 pb-5">
          <div>
            <div className="flex items-center gap-2 text-[11px] font-semibold text-text-muted">
              <span>Home</span><span>·</span><span>Lifecycle canon</span>
            </div>
            <h1 className="mt-1 text-[24px] font-semibold tracking-[-0.025em] text-text-primary">Stop workspaces and agents without removing them</h1>
            <p className="mt-1 max-w-3xl text-[12.5px] leading-relaxed text-text-secondary">
              A focused engineering lead pauses every runtime or one agent during an active work session. Identity and records stay visible; routing and runtime state remain unambiguous.
            </p>
          </div>
          <div className="shrink-0 space-y-1.5 text-right">
            <div className="flex items-center justify-end gap-1" aria-label="Workspace preview state">
              <span className="mr-1 text-[10.5px] font-semibold text-text-tertiary">Workspace</span>
              <div className="flex items-center gap-1 rounded-lg bg-fill p-1">
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
                    className={`rounded-md px-2 py-1 text-[10.5px] font-semibold transition-colors ${
                      workspacePreview === value ? "bg-surface-raised text-text-primary ring-1 ring-border" : "text-text-muted hover:text-text-primary"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex items-center justify-end gap-1" aria-label="Agent preview state">
              <span className="mr-1 text-[10.5px] font-semibold text-text-tertiary">Agent</span>
              <div className="flex items-center gap-1 rounded-lg bg-fill p-1">
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
                    className={`rounded-md px-2 py-1 text-[10.5px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-35 ${
                      preview === value ? "bg-surface-raised text-text-primary ring-1 ring-border" : "text-text-muted hover:text-text-primary"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </header>

        <WorkspaceFrame
          preview={preview}
          setPreview={setPreview}
          workspacePreview={workspacePreview}
          setWorkspacePreview={setWorkspacePreview}
          openStop={() => setDialog("stop")}
          openRemove={() => setDialog("remove")}
          openWorkspaceStop={() => setDialog("workspace-stop")}
        />

        <section className="mt-6 overflow-hidden rounded-[12px] bg-surface ring-1 ring-border">
          <div className="border-b border-border px-5 py-3">
            <h2 className="text-[14px] font-semibold text-text-primary">Workspace lifecycle states</h2>
            <p className="mt-0.5 text-[11.5px] text-text-secondary">Workspace availability controls every runtime without changing individual agent availability.</p>
          </div>
          <div className="flex divide-x divide-border">
            <StateSpecimen title="Stopped workspace" body="Retained content stays inspectable; no runtime starts on entry." state="stopped" action="Start workspace" />
            <StateSpecimen title="Starting" body="Launch only individually active agents and keep stopped agents stopped." state="stopped" action="Starting workspace…" busy />
            <StateSpecimen title="Partial launch" body="Keep successful agents active; name each failure and offer a scoped retry." state="running" action="Retry 1 agent" />
            <StateSpecimen title="Started workspace" body="Stop workspace is neutral lifecycle control, never a destructive Remove." state="running" action="Stop workspace" />
          </div>
        </section>

        <section className="mt-6 overflow-hidden rounded-[12px] bg-surface ring-1 ring-border">
          <div className="border-b border-border px-5 py-3">
            <h2 className="text-[14px] font-semibold text-text-primary">Lifecycle action states</h2>
            <p className="mt-0.5 text-[11.5px] text-text-secondary">Same compact component vocabulary; labels carry meaning without relying on colour.</p>
          </div>
          <div className="flex divide-x divide-border">
            <StateSpecimen title="Active · idle" body="Stop is neutral and distinct from Remove in More actions." state="running" action="Stop" />
            <StateSpecimen title="Active · working" body="Confirm before terminating current runtime and work; the confirm action becomes Stopping agent… while in flight." state="running" action="Stop agent" />
            <StateSpecimen title="Stopped" body="Stopped is persistent. Resume agent is the primary lifecycle action." state="stopped" action="Resume agent" />
            <StateSpecimen title="In flight" body="Keep Stopped visible until resume succeeds; disable duplicate actions." state="stopped" action="Resuming agent…" busy />
          </div>
        </section>

        <section className="mt-6 grid grid-cols-[360px_1fr] overflow-hidden rounded-[12px] bg-surface ring-1 ring-border">
          <RoutingSpecimen />
          <div className="border-l border-border px-6 py-4">
            <h2 className="text-[14px] font-semibold text-text-primary">Interaction contract</h2>
            <div className="mt-3 grid grid-cols-2 gap-x-8 gap-y-4 text-[11.5px] leading-relaxed">
              <div><strong className="block text-text-primary">Stop</strong><span className="text-text-secondary">Confirm every live runtime. The confirm action becomes “Stopping agent…” and disables duplicate input. On success, preserve selection and focus Resume agent. On failure, keep the runtime active, keep the dialog open, and show an inline alert.</span></div>
              <div><strong className="block text-text-primary">Resume</strong><span className="text-text-secondary">Use “Resume agent” for lifecycle copy. “Restore snapshot” names context recovery. While loading, keep the agent stopped and disable retries.</span></div>
              <div><strong className="block text-text-primary">Errors and announcements</strong><span className="text-text-secondary">Inline role=alert; no toast-only failures. Stop failure leaves the runtime active. Resume failure leaves it stopped. Success uses a polite aria-live announcement.</span></div>
              <div><strong className="block text-text-primary">Keyboard and focus</strong><span className="text-text-secondary">Row, Stop, Resume, and More actions are independently reachable. Escape returns to the trigger. Disabled routing options are skipped by selection and identify “Stopped” in their accessible name.</span></div>
              <div><strong className="block text-text-primary">Concurrent routing</strong><span className="text-text-secondary">If the selected recipient stops, reset Send to self (or the first active target) before the next send and announce the change. Never queue work to a stopped target.</span></div>
              <div><strong className="block text-text-primary">Removal</strong><span className="text-text-secondary">Remove remains under More actions, uses red destructive styling, and confirms separately. Never relabel Remove as Stop or place it in the blue primary action slot.</span></div>
              <div><strong className="block text-text-primary">Stopped workspace entry</strong><span className="text-text-secondary">Load retained navigation, roster, task, message, and history surfaces without auto-launch. Center the explicit Start workspace action; keep agent lifecycle actions unavailable until started.</span></div>
              <div><strong className="block text-text-primary">Workspace start</strong><span className="text-text-secondary">Start only individually active agents. Keep per-agent progress visible. Partial success keeps successful runtimes active, names failures inline, and offers Retry failed agents.</span></div>
              <div><strong className="block text-text-primary">Workspace stop</strong><span className="text-text-secondary">If any agent is working, confirm that all live runtimes and current work terminate immediately. “Stopping workspace…” disables duplicate input. Success preserves every record; failure names agents still running.</span></div>
            </div>
          </div>
        </section>

        <footer className="flex items-center justify-between py-5 text-[10.5px] text-text-tertiary">
          <span>Affected real-app view: home</span>
          <span>Implementation pixel gate: home/default and home/empty · open and inspect both PNGs</span>
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
