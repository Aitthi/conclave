import { Hexagon, FolderPlus, UsersRound } from "lucide-react";

import type { Workspace } from "../ipc";

// Default color for workspaces that have no color set.
const DEFAULT_WORKSPACE_COLOR = "#6e6e73";

interface RailProps {
  workspaces: Workspace[];
  activeWorkspaceId: string | null;
  onSelectWorkspace: (id: string) => void;
  onOpenLibrary?: () => void;
  onOpenLinkFolder?: () => void;
  onOpenSettings?: () => void;
}

export function Rail({ workspaces, activeWorkspaceId, onSelectWorkspace, onOpenLibrary, onOpenLinkFolder }: RailProps) {
  return (
    <nav className="w-[56px] shrink-0 bg-fill-soft border-r border-overlay/[0.06] flex flex-col items-center overflow-hidden">
      {/* Brand mark — sits in a 48px header zone so it lines up with the Roster
          and Context headers; its bottom border continues the toolbar rule
          across the Rail column. */}
      <div className="h-12 w-full shrink-0 grid place-items-center border-b border-overlay/[0.06] bg-sidebar">
        <button
          className="w-8 h-8 rounded-[9px] grid place-items-center"
          title="Conclave"
        >
          <Hexagon className="w-[18px] h-[18px] text-accent" />
        </button>
      </div>

      {/* Switcher + actions, below the brand header zone */}
      <div className="flex-1 w-full flex flex-col items-center gap-1 pt-2.5 pb-2.5 overflow-hidden">
      {/* Workspace switcher — driven by real DB data from workspace.list */}
      {workspaces.map((ws) => {
        const isActive = ws.id === activeWorkspaceId;
        const color = ws.color ?? DEFAULT_WORKSPACE_COLOR;
        const letter = ws.name.charAt(0).toUpperCase();

        return (
          <button
            key={ws.id}
            className={`relative w-9 h-9 rounded-[10px] text-white grid place-items-center text-[13px] font-bold ring-hair transition-opacity${
              isActive ? "" : " opacity-90 hover:opacity-100"
            }`}
            style={{ backgroundColor: color }}
            title={ws.name}
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
      </div>
      </div>
    </nav>
  );
}
