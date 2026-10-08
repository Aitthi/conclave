import type { WorkspaceStartBatch } from "../components/WorkspacePane";

/** True when the Start batch already ruled on this instance and it is NOT ready:
 *  it failed to launch or was skipped as stopped. An id the batch never saw
 *  (added after Start) is never blocked — the pane spawns it like any other
 *  active tab. */
export function blockedByStartBatch(
  batch: WorkspaceStartBatch | null | undefined,
  instanceId: string,
): boolean {
  if (!batch) return false;
  if (batch.readyAgentIds.includes(instanceId)) return false;
  return (
    batch.skippedStoppedAgentIds.includes(instanceId) ||
    batch.failures.some((f) => f.workspaceAgentId === instanceId)
  );
}
