import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, useAnyMessageInjected } from "../ipc";
import type { InterAgentMessage } from "../ipc";

// The hub loads the workspace's full recent window (the server clamps to its
// max); per-pair narrowing and search are client-side over this window.
const MESSAGE_LIMIT = 200;

export interface AgentIdentity {
  name: string;
  color: string;
  /** From `instance.list`'s resolved `roleName` (first-class role or legacy
   *  free-text label) — absent for a role-less agent. */
  role?: string;
}

export const FALLBACK_IDENTITY: AgentIdentity = { name: "unknown", color: "#8e8e93" };

/**
 * Shared workspace-chat data layer — identities + messages + live refetch.
 * Extracted from `ChatHub.tsx` (behavior-preserving); consumed by both the
 * Chat Hub and the right-rail `ChatRail`.
 */
export function useWorkspaceChat(workspaceId: string) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // ── Identities: instanceId → { name, color } (instance.list ⨝ agentDef.list,
  //    the same join WorkspacePane/Blackboard do). ─────────────────────────────
  const [agents, setAgents] = useState<Map<string, AgentIdentity>>(new Map());
  useEffect(() => {
    let active = true;
    Promise.all([ipc.instance.list({ workspaceId }), ipc.agentDef.list()])
      .then(([instances, defs]) => {
        if (!active) return;
        const defsById = new Map(defs.map((d) => [d.id, d]));
        const m = new Map<string, AgentIdentity>();
        for (const inst of instances) {
          const def = defsById.get(inst.agentDefId);
          if (def) m.set(inst.id, { name: def.name, color: def.color ?? "#6e6e73", role: inst.roleName });
        }
        setAgents(m);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("useWorkspaceChat: identity load failed", err);
      });
    return () => {
      active = false;
    };
  }, [workspaceId]);
  const identityOf = useCallback(
    (id: string): AgentIdentity => agents.get(id) ?? FALLBACK_IDENTITY,
    [agents],
  );

  // ── Messages — REAL data from message.listForWorkspace, newest-first from
  //    the API; seq-guarded so a stale response can't overwrite a newer one. ──
  const [messages, setMessages] = useState<InterAgentMessage[]>([]);
  const [loadError, setLoadError] = useState(false);
  const seq = useRef(0);
  const refetch = useCallback(() => {
    const mine = ++seq.current;
    ipc.message
      .listForWorkspace({ workspaceId, limit: MESSAGE_LIMIT })
      .then((rows) => {
        if (mounted.current && mine === seq.current) {
          setMessages(rows);
          setLoadError(false);
        }
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("useWorkspaceChat: listForWorkspace failed", err);
        if (mounted.current && mine === seq.current) setLoadError(true);
      });
  }, [workspaceId]);
  useEffect(() => {
    refetch();
  }, [refetch]);
  // Any injection anywhere → refetch (workspace-scoped server-side; a
  // cross-workspace event costs one cheap guarded refetch).
  useAnyMessageInjected(() => refetch());

  return { messages, loadError, identityOf, refetch };
}
