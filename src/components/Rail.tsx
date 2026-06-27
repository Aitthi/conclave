import { useState } from "react";
import {
  Hexagon,
  FolderPlus,
  SquareTerminal,
  UsersRound,
  Settings,
} from "lucide-react";

interface Workspace {
  id: string;
  label: string;
  color: string;
  title: string;
}

const WORKSPACES: Workspace[] = [
  { id: "codeup", label: "C", color: "#0a84ff", title: "codeup" },
  { id: "payments", label: "P", color: "#0fa3a3", title: "payments-svc" },
  { id: "marketing", label: "M", color: "#5e5ce6", title: "marketing-site" },
];

export function Rail() {
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string>("codeup");

  return (
    <nav className="w-[56px] shrink-0 bg-[#ebebed] border-r border-black/[0.06] flex flex-col items-center py-2.5 gap-1 overflow-hidden">
      {/* Brand mark */}
      <button
        className="w-8 h-8 rounded-[9px] grid place-items-center mb-1"
        title="Conclave"
      >
        <Hexagon className="w-[18px] h-[18px] text-[#0a84ff]" />
      </button>

      {/* Divider */}
      <div className="w-7 h-px bg-black/[0.08] mb-1.5" />

      {/* Workspace switcher */}
      {WORKSPACES.map((ws) => {
        const isActive = ws.id === activeWorkspaceId;
        return (
          <button
            key={ws.id}
            className={`relative w-9 h-9 rounded-[10px] text-white grid place-items-center text-[13px] font-bold ring-hair transition-opacity${
              isActive ? "" : " opacity-90 hover:opacity-100"
            }`}
            style={{ backgroundColor: ws.color }}
            title={ws.title}
            onClick={() => setActiveWorkspaceId(ws.id)}
          >
            {/* Active selection pill */}
            {isActive && (
              <span
                className="absolute -left-2.5 top-1/2 -translate-y-1/2 w-1 h-5 rounded-full"
                style={{ backgroundColor: ws.color }}
              />
            )}
            {ws.label}
          </button>
        );
      })}

      {/* Link folder / new workspace */}
      <button
        className="w-9 h-9 rounded-[10px] border border-dashed border-black/20 text-[#6e6e73] grid place-items-center hover:border-[#0a84ff] hover:text-[#0a84ff]"
        title="Link folder as workspace"
      >
        <FolderPlus className="w-[17px] h-[17px]" />
      </button>

      {/* Bottom actions */}
      <div className="mt-auto flex flex-col items-center gap-1.5">
        <button
          className="w-9 h-9 rounded-[10px] text-[#6e6e73] grid place-items-center hover:bg-black/[0.05]"
          title="Conclave CLI"
        >
          <SquareTerminal className="w-[17px] h-[17px]" />
        </button>
        <button
          className="w-9 h-9 rounded-[10px] bg-white ring-hair text-[#3a3a3c] grid place-items-center hover:bg-black/[0.03]"
          title="Agent Library"
        >
          <UsersRound className="w-[17px] h-[17px]" />
        </button>
        <button
          className="w-9 h-9 rounded-[10px] text-[#6e6e73] grid place-items-center hover:bg-black/[0.05]"
          title="Settings"
        >
          <Settings className="w-[17px] h-[17px]" />
        </button>
      </div>
    </nav>
  );
}
