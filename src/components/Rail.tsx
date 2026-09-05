import type { ReactNode } from "react";
import {
  CirclePause,
  FolderCog,
  Globe,
  FolderPlus,
  Package,
  PenTool,
  UsersRound,
  Wand2,
} from "lucide-react";

import type { Workspace } from "../ipc";

// Default color for workspaces that have no color set.
const DEFAULT_WORKSPACE_COLOR = "#6e6e73";

interface RailProps {
  workspaces: Workspace[];
  activeWorkspaceId: string | null;
  globalDestination?: "overview" | "workspaces" | null;
  onSelectWorkspace: (id: string) => void;
  onOpenOverview?: () => void;
  onOpenWorkspaces?: () => void;
  artifactsOpen?: boolean;
  designOpen?: boolean;
  browserOpen?: boolean;
  browserActive?: boolean;
  onOpenArtifacts?: () => void;
  onOpenDesign?: () => void;
  onOpenBrowser?: () => void;
  onOpenLibrary?: () => void;
  onOpenSkillLibrary?: () => void;
  onOpenLinkFolder?: () => void;
  onOpenSettings?: () => void;
}

function RailActionButton({
  active,
  disabled,
  title,
  onClick,
  children,
}: {
  active?: boolean;
  disabled?: boolean;
  title: string;
  onClick?: () => void;
  children: ReactNode;
}) {
  return (
    <button
      className={`relative w-9 h-9 rounded-[10px] grid place-items-center ring-hair transition-colors ${
        active
          ? "bg-accent/12 text-accent"
          : "bg-surface text-text-body hover:bg-overlay/[0.03]"
      } ${disabled ? "opacity-45 cursor-default hover:bg-surface" : ""}`}
      title={title}
      aria-pressed={active}
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
    >
      {active && (
        <span className="absolute -left-2.5 top-1/2 -translate-y-1/2 w-1 h-5 rounded-full bg-accent" />
      )}
      {children}
    </button>
  );
}

export function Rail({
  workspaces,
  activeWorkspaceId,
  globalDestination,
  onSelectWorkspace,
  onOpenOverview,
  onOpenWorkspaces,
  artifactsOpen,
  designOpen,
  browserOpen,
  browserActive,
  onOpenArtifacts,
  onOpenDesign,
  onOpenBrowser,
  onOpenLibrary,
  onOpenSkillLibrary,
  onOpenLinkFolder,
}: RailProps) {
  const workspaceScopedDisabled = activeWorkspaceId == null;

  return (
    <nav className="w-[56px] shrink-0 bg-fill-soft border-r border-overlay/[0.06] flex flex-col items-center overflow-hidden">
      {/* Brand mark — sits in a 48px header zone so it lines up with the Roster
          and Context headers; its bottom border continues the toolbar rule
          across the Rail column. */}
      <div className="h-12 w-full shrink-0 grid place-items-center border-b border-overlay/[0.06] bg-sidebar">
        <button
          className={`relative w-8 h-8 rounded-[9px] grid place-items-center transition-colors ${
            globalDestination === "overview" ? "bg-accent/10" : "hover:bg-overlay/[0.04]"
          }`}
          title="Overview"
          aria-label="Overview"
          aria-current={globalDestination === "overview" ? "page" : undefined}
          onClick={onOpenOverview}
        >
          {globalDestination === "overview" && (
            <span className="absolute -left-3 top-1/2 h-5 w-1 -translate-y-1/2 rounded-full bg-accent" />
          )}
          <img src="/brand/logo-mark.svg" alt="" className="h-[19px] w-[19px]" />
        </button>
      </div>

      {/* Switcher + actions, below the brand header zone */}
      <div className="flex-1 w-full flex flex-col items-center gap-1 pt-2.5 pb-2.5 overflow-hidden">
        <RailActionButton
          active={globalDestination === "workspaces"}
          title="Manage workspaces"
          onClick={onOpenWorkspaces}
        >
          <FolderCog className="h-[17px] w-[17px]" />
        </RailActionButton>

        {/* Workspace switcher — driven by real DB data from workspace.list */}
        {workspaces.map((ws) => {
          const isActive = globalDestination == null && ws.id === activeWorkspaceId;
          const color = ws.color ?? DEFAULT_WORKSPACE_COLOR;
          const letter = ws.name.charAt(0).toUpperCase();

          return (
            <button
              key={ws.id}
              className={`relative w-9 h-9 rounded-[10px] text-white grid place-items-center text-[13px] font-bold ring-hair transition-opacity${
                isActive ? "" : " opacity-90 hover:opacity-100"
              }`}
              style={{ backgroundColor: color }}
              title={`${ws.name} · ${ws.runState}`}
              aria-label={`${ws.name}, workspace ${ws.runState}`}
              onClick={() => onSelectWorkspace(ws.id)}
            >
              {/* Active selection pill */}
              {isActive && (
                <span
                  className="absolute -left-2.5 top-1/2 -translate-y-1/2 w-1 h-5 rounded-full"
                  style={{ backgroundColor: color }}
                />
              )}
              {letter}
              {ws.runState === "stopped" && (
                <span className="absolute -bottom-1 -right-1 grid h-4 w-4 place-items-center rounded-full bg-sidebar text-text-secondary ring-1 ring-overlay/15">
                  <CirclePause className="h-2.5 w-2.5" aria-hidden="true" />
                </span>
              )}
            </button>
          );
        })}

        {/* Link folder / new workspace */}
        <button
          className="w-9 h-9 rounded-[10px] border border-dashed border-overlay/20 text-text-secondary grid place-items-center hover:border-accent hover:text-accent"
          title="Link folder as workspace"
          onClick={onOpenLinkFolder}
        >
          <FolderPlus className="w-[17px] h-[17px]" />
        </button>

        <div className="mt-2 flex flex-col items-center gap-1.5">
          <RailActionButton
            active={!!designOpen}
            disabled={workspaceScopedDisabled}
            title="Design view"
            onClick={onOpenDesign}
          >
            <PenTool className="w-[17px] h-[17px]" />
          </RailActionButton>
          <RailActionButton
            active={!!artifactsOpen}
            disabled={workspaceScopedDisabled}
            title="Artifacts view"
            onClick={onOpenArtifacts}
          >
            <Package className="w-[17px] h-[17px]" />
          </RailActionButton>
          <div className="relative">
            <RailActionButton
              active={!!browserOpen}
              disabled={workspaceScopedDisabled}
              title="Browser view"
              onClick={onOpenBrowser}
            >
              <Globe className="w-[17px] h-[17px]" />
            </RailActionButton>
            {browserActive && !browserOpen && (
              <span
                className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-success ring-2 ring-fill-soft"
                title="A browser is open"
              />
            )}
          </div>
        </div>

        {/* Bottom actions. Conclave CLI + Settings are hidden for now until they
            have real behavior. */}
        <div className="mt-auto flex flex-col items-center gap-1.5">
          <button
            className="w-9 h-9 rounded-[10px] bg-surface ring-hair text-text-body grid place-items-center hover:bg-overlay/[0.03]"
            title="Agent Library"
            onClick={onOpenLibrary}
          >
            <UsersRound className="w-[17px] h-[17px]" />
          </button>
          <button
            className="w-9 h-9 rounded-[10px] bg-surface ring-hair text-text-body grid place-items-center hover:bg-overlay/[0.03]"
            title="Skill Library"
            onClick={onOpenSkillLibrary}
          >
            <Wand2 className="w-[17px] h-[17px]" />
          </button>
        </div>
      </div>
    </nav>
  );
}
