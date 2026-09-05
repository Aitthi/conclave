import { useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  CheckCircle2,
  CirclePause,
  FolderPlus,
  MoreHorizontal,
  Search,
  Square,
  X,
} from "lucide-react";

import type { Workspace } from "../ipc";
import "./workspace-manager.css";

type WorkspaceTab = "active" | "archived";
type WorkspaceNotice = {
  kind: "archived" | "restored";
  workspace: Workspace;
};

export interface WorkspaceManagerProps {
  activeWorkspaces: Workspace[];
  archivedWorkspaces: Workspace[];
  loading: boolean;
  error: string | null;
  navigationError?: string | null;
  initialTab?: WorkspaceTab;
  notice?: WorkspaceNotice | null;
  onRetry: () => void;
  onOpen: (id: string) => Promise<void>;
  onManage: (id: string) => void;
  onLink: () => void;
  onRestore: (id: string) => Promise<Workspace>;
  onDismissNotice: () => void;
  onUndoArchive: (id: string) => Promise<Workspace>;
}

type RowAction = { kind: "open" | "restore"; workspaceId: string };
type NoticeAction = "open" | "undo";

function detailFrom(error: unknown): string {
  const detail = error instanceof Error ? error.message.trim() : String(error).trim();
  return detail ? ` ${detail}` : "";
}

function workspaceMatches(workspace: Workspace, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  return `${workspace.name} ${workspace.folderPath}`.toLocaleLowerCase().includes(needle);
}

function WorkspaceStatus({ workspace, archived }: { workspace: Workspace; archived: boolean }) {
  if (archived) {
    return (
      <span className="workspace-manager-status">
        <Archive aria-hidden="true" />
        Archived
      </span>
    );
  }

  if (workspace.runState === "started") {
    return (
      <span className="workspace-manager-status">
        <Square aria-hidden="true" />
        Started
      </span>
    );
  }

  return (
    <span className="workspace-manager-status">
      <CirclePause aria-hidden="true" />
      Stopped
    </span>
  );
}

export function WorkspaceManager({
  activeWorkspaces,
  archivedWorkspaces,
  loading,
  error,
  navigationError = null,
  initialTab = "active",
  notice = null,
  onRetry,
  onOpen,
  onManage,
  onLink,
  onRestore,
  onDismissNotice,
  onUndoArchive,
}: WorkspaceManagerProps) {
  const [tab, setTab] = useState<WorkspaceTab>(initialTab);
  const [query, setQuery] = useState("");
  const [expandedWorkspaceId, setExpandedWorkspaceId] = useState<string | null>(null);
  const [pendingRowAction, setPendingRowAction] = useState<RowAction | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [localNotice, setLocalNotice] = useState<WorkspaceNotice | null>(null);
  const [pendingNoticeAction, setPendingNoticeAction] = useState<NoticeAction | null>(null);
  const [noticeError, setNoticeError] = useState<string | null>(null);
  const noticeActionRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    setTab(initialTab);
  }, [initialTab]);

  useEffect(() => {
    document.body.dataset.conclaveState = loading ? "loading" : error ? "error" : "ready";
    return () => {
      delete document.body.dataset.conclaveState;
    };
  }, [error, loading]);

  useEffect(() => {
    if (!expandedWorkspaceId) return;
    const stillPresent = [...activeWorkspaces, ...archivedWorkspaces].some(
      (workspace) => workspace.id === expandedWorkspaceId,
    );
    if (!stillPresent) setExpandedWorkspaceId(null);
  }, [activeWorkspaces, archivedWorkspaces, expandedWorkspaceId]);

  const visibleNotice = notice ?? localNotice;

  useEffect(() => {
    if (visibleNotice) noticeActionRef.current?.focus();
  }, [visibleNotice]);

  const selectedWorkspaces = tab === "active" ? activeWorkspaces : archivedWorkspaces;
  const filteredWorkspaces = useMemo(
    () => selectedWorkspaces.filter((workspace) => workspaceMatches(workspace, query)),
    [query, selectedWorkspaces],
  );

  function changeTab(nextTab: WorkspaceTab) {
    setTab(nextTab);
    setExpandedWorkspaceId(null);
  }

  function clearRowError(workspaceId: string) {
    setRowErrors((current) => {
      if (!(workspaceId in current)) return current;
      const next = { ...current };
      delete next[workspaceId];
      return next;
    });
  }

  async function runRowAction(workspace: Workspace, kind: RowAction["kind"]) {
    if (pendingRowAction) return;
    setPendingRowAction({ kind, workspaceId: workspace.id });
    clearRowError(workspace.id);

    try {
      if (kind === "restore") {
        const restored = await onRestore(workspace.id);
        setLocalNotice({ kind: "restored", workspace: restored });
        setExpandedWorkspaceId(null);
      } else {
        await onOpen(workspace.id);
      }
    } catch (actionError) {
      const message = kind === "restore"
        ? "Couldn’t restore. This workspace remains archived."
        : `Couldn’t open this workspace. It remains unchanged.${detailFrom(actionError)}`;
      setRowErrors((current) => ({ ...current, [workspace.id]: message }));
    } finally {
      setPendingRowAction(null);
    }
  }

  function dismissNotice() {
    if (pendingNoticeAction) return;
    setLocalNotice(null);
    setNoticeError(null);
    onDismissNotice();
  }

  async function runNoticeAction(kind: NoticeAction) {
    if (!visibleNotice || pendingNoticeAction) return;
    setPendingNoticeAction(kind);
    setNoticeError(null);

    try {
      if (kind === "undo") {
        const restored = await onUndoArchive(visibleNotice.workspace.id);
        setLocalNotice({ kind: "restored", workspace: restored });
        onDismissNotice();
      } else {
        await onOpen(visibleNotice.workspace.id);
        setLocalNotice(null);
        onDismissNotice();
      }
    } catch (actionError) {
      setNoticeError(
        kind === "undo"
          ? `Couldn’t undo archive. The workspace remains archived.${detailFrom(actionError)}`
          : `Couldn’t open this workspace. It remains stopped.${detailFrom(actionError)}`,
      );
    } finally {
      setPendingNoticeAction(null);
    }
  }

  const hasQuery = query.trim().length > 0;
  const panelLabel = tab === "active" ? "Active workspaces" : "Archived workspaces";

  return (
    <main className="workspace-manager" aria-labelledby="workspace-manager-title">
      <header className="workspace-manager-header">
        <div>
          <h1 id="workspace-manager-title">Workspaces</h1>
          <p>Manage project folders and retained workspaces</p>
        </div>
        <button type="button" className="workspace-manager-button is-primary" onClick={onLink}>
          <FolderPlus aria-hidden="true" />
          New workspace
        </button>
      </header>

      <div className="workspace-manager-content">
        {navigationError && (
          <div className="workspace-manager-navigation-error" role="alert">
            {navigationError}
          </div>
        )}
        <div className="workspace-manager-controls">
          <div>
            <div className="workspace-manager-tabs" role="tablist" aria-label="Workspace filters">
              <button
                type="button"
                role="tab"
                aria-selected={tab === "active"}
                aria-controls="workspace-manager-panel"
                className={tab === "active" ? "is-selected" : undefined}
                onClick={() => changeTab("active")}
              >
                Active <span>{loading || error ? "—" : activeWorkspaces.length}</span>
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={tab === "archived"}
                aria-controls="workspace-manager-panel"
                className={tab === "archived" ? "is-selected" : undefined}
                onClick={() => changeTab("archived")}
              >
                Archived <span>{loading || error ? "—" : archivedWorkspaces.length}</span>
              </button>
            </div>
            <p className="workspace-manager-tab-help">
              {tab === "active"
                ? "Includes started and stopped workspaces."
                : "Restore brings a workspace back stopped. Nothing launches."}
            </p>
          </div>

          <label className="workspace-manager-search">
            <span className="sr-only">Search workspace name or folder path</span>
            <Search aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search name or path"
            />
          </label>
        </div>

        <section
          id="workspace-manager-panel"
          role="tabpanel"
          aria-label={panelLabel}
          aria-busy={loading}
          className="workspace-manager-list"
        >
          <div className="workspace-manager-list-header" aria-hidden="true">
            <span>Workspace / folder</span>
            <span>Status</span>
            <span>Actions</span>
          </div>

          {loading ? (
            <div className="workspace-manager-loading" aria-label="Loading workspaces">
              <i />
              <i />
              <i />
            </div>
          ) : error ? (
            <div className="workspace-manager-state is-error" role="alert">
              <h2>Couldn’t load workspaces</h2>
              <p>Existing workspaces and running agents are unchanged.</p>
              <button type="button" className="workspace-manager-button" onClick={onRetry}>
                Retry
              </button>
            </div>
          ) : filteredWorkspaces.length === 0 ? (
            <div className="workspace-manager-state">
              <Archive aria-hidden="true" className="workspace-manager-state-icon" />
              <h2>
                {hasQuery
                  ? "No matching workspaces"
                  : tab === "archived"
                    ? "No archived workspaces"
                    : archivedWorkspaces.length > 0
                      ? "All workspaces are archived"
                      : "Link your first workspace"}
              </h2>
              <p>
                {hasQuery
                  ? "Try another name or folder path."
                  : tab === "archived"
                    ? "Archive keeps agents, sessions, tasks, memory, artifacts and project files."
                    : "Link a project folder or restore a workspace to return it to the Rail."}
              </p>
              <button
                type="button"
                className="workspace-manager-button"
                onClick={() => {
                  if (hasQuery) setQuery("");
                  else if (tab === "archived") changeTab("active");
                  else if (archivedWorkspaces.length > 0) changeTab("archived");
                  else onLink();
                }}
              >
                {hasQuery
                  ? "Clear search"
                  : tab === "archived"
                    ? "Back to Active"
                    : archivedWorkspaces.length > 0
                      ? "View archived"
                      : "Link folder"}
              </button>
            </div>
          ) : (
            filteredWorkspaces.map((workspace) => {
              const archived = tab === "archived";
              const pending = pendingRowAction?.workspaceId === workspace.id;
              const actionLabel = archived
                ? pending && pendingRowAction.kind === "restore" ? "Restoring…" : "Restore"
                : pending && pendingRowAction.kind === "open" ? "Opening…" : "Open";
              const expanded = expandedWorkspaceId === workspace.id;

              return (
                <article className="workspace-manager-row-wrap" data-workspace-id={workspace.id} key={workspace.id}>
                  <div className="workspace-manager-row">
                    <div className="workspace-manager-identity">
                      <span
                        className={`workspace-manager-avatar${workspace.color ? " has-color" : ""}`}
                        style={workspace.color ? { backgroundColor: workspace.color } : undefined}
                        aria-hidden="true"
                      >
                        {workspace.name.charAt(0).toLocaleUpperCase() || "W"}
                      </span>
                      <div>
                        <h2 title={workspace.name}>{workspace.name}</h2>
                        <p title={workspace.folderPath}>{workspace.folderPath}</p>
                      </div>
                    </div>

                    <WorkspaceStatus workspace={workspace} archived={archived} />

                    <div className="workspace-manager-row-actions">
                      <button
                        type="button"
                        data-workspace-action={archived ? "restore" : "open"}
                        className={`workspace-manager-button${archived ? " is-primary" : ""}`}
                        disabled={pendingRowAction !== null}
                        onClick={() => void runRowAction(workspace, archived ? "restore" : "open")}
                      >
                        {actionLabel}
                      </button>
                      <button
                        type="button"
                        data-workspace-action="menu"
                        className="workspace-manager-icon-button"
                        aria-label={`Manage ${workspace.name}`}
                        aria-expanded={expanded}
                        disabled={pendingRowAction !== null}
                        onClick={() => setExpandedWorkspaceId(expanded ? null : workspace.id)}
                      >
                        <MoreHorizontal aria-hidden="true" />
                      </button>
                    </div>
                  </div>

                  {rowErrors[workspace.id] && (
                    <div className="workspace-manager-row-alert" role="alert">
                      <span>{rowErrors[workspace.id]}</span>
                      <button
                        type="button"
                        data-workspace-action="settings"
                        disabled={pendingRowAction !== null}
                        onClick={() => void runRowAction(workspace, archived ? "restore" : "open")}
                      >
                        Retry
                      </button>
                    </div>
                  )}

                  {expanded && (
                    <div className="workspace-manager-manage-strip">
                      <span>
                        {archived
                          ? "Restore before editing or opening. Permanent deletion is separate."
                          : workspace.runState === "started"
                            ? "Stop workspace before archiving, even when no agents are working."
                            : "Archive retains all records and project files."}
                      </span>
                      <button
                        type="button"
                        className="workspace-manager-button"
                        onClick={() => onManage(workspace.id)}
                      >
                        Manage workspace
                      </button>
                    </div>
                  )}
                </article>
              );
            })
          )}
        </section>

        <p className="workspace-manager-retention-note">
          Archived workspaces leave the normal Rail and list. Agents, sessions, tasks, memory,
          artifacts and project files are retained.
        </p>
      </div>

      {visibleNotice && (
        <div className="workspace-manager-notice" role="status" aria-live="polite" aria-atomic="true">
          <CheckCircle2 aria-hidden="true" className="workspace-manager-notice-icon" />
          <div className="workspace-manager-notice-copy">
            <span>
              {visibleNotice.kind === "archived"
                ? `${visibleNotice.workspace.name} archived. All records and files are retained.`
                : `${visibleNotice.workspace.name} restored. It remains stopped.`}
            </span>
            {noticeError && <span className="workspace-manager-notice-error">{noticeError}</span>}
          </div>
          <button
            ref={noticeActionRef}
            type="button"
            className="workspace-manager-notice-action"
            disabled={pendingNoticeAction !== null}
            onClick={() => void runNoticeAction(visibleNotice.kind === "archived" ? "undo" : "open")}
          >
            {pendingNoticeAction === "undo"
              ? "Restoring…"
              : pendingNoticeAction === "open"
                ? "Opening…"
                : visibleNotice.kind === "archived" ? "Undo" : "Open"}
          </button>
          <button
            type="button"
            className="workspace-manager-notice-dismiss"
            aria-label="Dismiss notification"
            disabled={pendingNoticeAction !== null}
            onClick={dismissNotice}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      )}
    </main>
  );
}
