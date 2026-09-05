import type { Workspace, WorkspaceAgent } from "../../ipc/types";
import type { FixtureHandlers } from "../backend";
import { emitFixtureEvent } from "../events";

export type WorkspaceFixtureVariant =
  | "default"
  | "empty"
  | "all-archived"
  | "loading"
  | "error"
  | "archive-error"
  | "archive-pending"
  | "restore-error"
  | "restore-pending"
  | "started"
  | "busy";

type WorkspaceFixtureSeed = {
  active: Workspace[];
  archived?: Workspace[];
  agents?: WorkspaceAgent[];
  variant?: WorkspaceFixtureVariant;
};

export type WorkspaceFixture = {
  handlers: FixtureHandlers;
  getWorkspace(id: string): Workspace | undefined;
  getAgents(): WorkspaceAgent[];
  updateAgents(update: (agents: WorkspaceAgent[]) => WorkspaceAgent[]): void;
};

const ARCHIVED_AT = "2026-09-05T09:00:00.000Z";
const LINKED_AT = "2026-09-05T09:05:00.000Z";

function copyWorkspace(workspace: Workspace): Workspace {
  return { ...workspace };
}

function copyAgent(agent: WorkspaceAgent): WorkspaceAgent {
  return { ...agent };
}

function archiveOrder(a: Workspace, b: Workspace): number {
  const byTime = (b.archivedAt ?? "").localeCompare(a.archivedAt ?? "");
  return byTime || a.id.localeCompare(b.id);
}

/**
 * A page-local workspace repository for fixture mode. All lifecycle handlers
 * below read and write these same arrays so list refetches and
 * `workspace:changed` subscribers observe the mutation that just succeeded.
 */
export function createWorkspaceFixture(seed: WorkspaceFixtureSeed): WorkspaceFixture {
  const variant = seed.variant ?? "default";
  let active = seed.active.map(copyWorkspace);
  let archived = (seed.archived ?? []).map(copyWorkspace).sort(archiveOrder);
  let agents = (seed.agents ?? []).map(copyAgent);
  let linkedSequence = 0;

  const getWorkspace = (id: string) =>
    active.find((workspace) => workspace.id === id)
    ?? archived.find((workspace) => workspace.id === id);

  const emitChanged = (workspace: Workspace) => {
    emitFixtureEvent("workspace:changed", {
      workspaceId: workspace.id,
      runState: workspace.runState,
      archivedAt: workspace.archivedAt ?? null,
    });
  };

  const handlers: FixtureHandlers = {
    "workspace.list": () => {
      if (variant === "loading") return new Promise<Workspace[]>(() => {});
      if (variant === "error") throw new Error("Workspace lists are temporarily unavailable.");
      return active.map(copyWorkspace);
    },
    "workspace.listArchived": () => {
      if (variant === "loading") return new Promise<Workspace[]>(() => {});
      if (variant === "error") throw new Error("Workspace lists are temporarily unavailable.");
      return archived.map(copyWorkspace);
    },
    "workspace.use": ({ workspaceId }) => {
      const workspace = getWorkspace(workspaceId);
      if (!workspace) throw new Error(`Workspace ${workspaceId} was not found.`);
      if (workspace.archivedAt) throw new Error("Restore this workspace before opening it.");
      return undefined;
    },
    "workspace.link": ({ folderPath, name, color }) => {
      linkedSequence += 1;
      const pathParts = folderPath.split(/[\\/]/).filter(Boolean);
      const workspace: Workspace = {
        id: `fx-ws-linked-${linkedSequence}`,
        name: name?.trim() || pathParts[pathParts.length - 1] || "linked-workspace",
        folderPath,
        ...(color ? { color } : {}),
        runState: "stopped",
        archivedAt: null,
        createdAt: LINKED_AT,
      };
      active = [...active, workspace];
      emitChanged(workspace);
      return { workspace: copyWorkspace(workspace), agents: [] };
    },
    "workspace.update": ({ workspaceId, name, color }) => {
      const index = active.findIndex((workspace) => workspace.id === workspaceId);
      if (index < 0) {
        if (archived.some((workspace) => workspace.id === workspaceId)) {
          throw new Error("Restore this workspace before changing its settings.");
        }
        throw new Error(`Workspace ${workspaceId} was not found.`);
      }
      const updated: Workspace = {
        ...active[index],
        name,
        ...(color === undefined ? {} : { color }),
      };
      active = active.map((workspace, candidate) => candidate === index ? updated : workspace);
      emitChanged(updated);
      return copyWorkspace(updated);
    },
    "workspace.start": ({ workspaceId }) => {
      const index = active.findIndex((workspace) => workspace.id === workspaceId);
      if (index < 0) throw new Error("Restore this workspace before starting it.");
      const workspace: Workspace = { ...active[index], runState: "started", archivedAt: null };
      active = active.map((candidate, candidateIndex) => candidateIndex === index ? workspace : candidate);
      const workspaceAgents = agents.filter((agent) => agent.workspaceId === workspaceId);
      const readyAgentIds = workspaceAgents
        .filter((agent) => agent.availability === "active")
        .map((agent) => agent.id);
      const skippedStoppedAgentIds = workspaceAgents
        .filter((agent) => agent.availability === "stopped")
        .map((agent) => agent.id);
      emitChanged(workspace);
      return { workspace: copyWorkspace(workspace), readyAgentIds, skippedStoppedAgentIds, failures: [] };
    },
    "workspace.stop": ({ workspaceId }) => {
      const index = active.findIndex((workspace) => workspace.id === workspaceId);
      if (index < 0) throw new Error("Restore this workspace before stopping it.");
      const stoppedRuntimeIds = agents
        .filter(
          (agent) =>
            agent.workspaceId === workspaceId
            && agent.availability === "active"
            && agent.status === "running",
        )
        .map((agent) => agent.id);
      agents = agents.map((agent) => agent.workspaceId === workspaceId
        ? { ...agent, status: "idle", working: false, sessionId: undefined }
        : agent);
      const workspace: Workspace = { ...active[index], runState: "stopped", archivedAt: null };
      active = active.map((candidate, candidateIndex) => candidateIndex === index ? workspace : candidate);
      emitChanged(workspace);
      return { workspace: copyWorkspace(workspace), stoppedRuntimeIds };
    },
    "workspace.archive": ({ workspaceId }) => {
      const alreadyArchived = archived.find((workspace) => workspace.id === workspaceId);
      if (alreadyArchived) return copyWorkspace(alreadyArchived);
      const index = active.findIndex((workspace) => workspace.id === workspaceId);
      if (index < 0) throw new Error(`Workspace ${workspaceId} was not found.`);
      const current = active[index];
      if (current.runState === "started") throw new Error("Stop workspace before archiving.");
      if (variant === "archive-pending") return new Promise<Workspace>(() => {});
      if (variant === "busy") throw new Error("Workspace is busy. Try archiving again when the current run finishes.");
      if (variant === "archive-error") throw new Error("Archive could not be completed. The workspace is unchanged.");
      const workspace: Workspace = { ...current, runState: "stopped", archivedAt: ARCHIVED_AT };
      active = active.filter((candidate) => candidate.id !== workspaceId);
      archived = [...archived, workspace].sort(archiveOrder);
      agents = agents.map((agent) => agent.workspaceId === workspaceId
        ? { ...agent, status: "idle", working: false, sessionId: undefined }
        : agent);
      emitChanged(workspace);
      return copyWorkspace(workspace);
    },
    "workspace.restore": ({ workspaceId }) => {
      const alreadyActive = active.find((workspace) => workspace.id === workspaceId);
      if (alreadyActive) return copyWorkspace(alreadyActive);
      const index = archived.findIndex((workspace) => workspace.id === workspaceId);
      if (index < 0) throw new Error(`Workspace ${workspaceId} was not found.`);
      if (variant === "restore-pending") return new Promise<Workspace>(() => {});
      if (variant === "restore-error") throw new Error("Restore could not be completed. The workspace remains archived.");
      const workspace: Workspace = {
        ...archived[index],
        runState: "stopped",
        archivedAt: null,
      };
      archived = archived.filter((candidate) => candidate.id !== workspaceId);
      active = [...active, workspace];
      emitChanged(workspace);
      return copyWorkspace(workspace);
    },
    "workspace.delete": ({ workspaceId }) => {
      const workspace = getWorkspace(workspaceId);
      if (!workspace) throw new Error(`Workspace ${workspaceId} was not found.`);
      active = active.filter((candidate) => candidate.id !== workspaceId);
      archived = archived.filter((candidate) => candidate.id !== workspaceId);
      agents = agents.filter((agent) => agent.workspaceId !== workspaceId);
      emitChanged(workspace);
      return undefined;
    },
  };

  return {
    handlers,
    getWorkspace,
    getAgents: () => agents.map(copyAgent),
    updateAgents(update) {
      agents = update(agents.map(copyAgent)).map(copyAgent);
    },
  };
}
