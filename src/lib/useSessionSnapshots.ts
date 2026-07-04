import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, useSnapshotCreated } from "../ipc";
import type { Snapshot } from "../ipc";

// How many recent snapshots the Memory section shows.
const SNAPSHOT_LOG_LIMIT = 6;

export interface SessionSnapshots {
  /** Newest-first, capped at SNAPSHOT_LOG_LIMIT. */
  snapshots: Snapshot[];
  /** Whether ANY handoff snapshot exists for this session (computed from the
   *  FULL list, before the display slice) — gates Session's Resume button. */
  hasHandoff: boolean;
  snapshotError: boolean;
  setSnapshotError: (v: boolean) => void;
  refetch: () => void;
}

/**
 * Single shared fetcher for a session's snapshot list + handoff flag —
 * extracted from `ContextDrawer.tsx` (lines 281-332, behavior-preserving).
 * `WorkspacePane` holds ONE instance per active session and passes it to
 * both the top bar (Resume gates on `hasHandoff`) and the bottom bar
 * (snapshot popover) — two hook calls would mean two fetchers racing each
 * other over the same list (plan-review F2).
 */
export function useSessionSnapshots(sessionId: string): SessionSnapshots {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [hasHandoff, setHasHandoff] = useState(false);
  const [snapshotError, setSnapshotError] = useState(false);
  const seq = useRef(0);

  const refetch = useCallback(() => {
    if (!sessionId) {
      setSnapshots([]);
      return;
    }
    const mine = ++seq.current;
    ipc.snapshot
      .list({ sessionId })
      .then((rows) => {
        if (mounted.current && mine === seq.current) {
          setSnapshots(rows.slice(0, SNAPSHOT_LOG_LIMIT));
          setHasHandoff(rows.some((r) => r.type === "handoff"));
          setSnapshotError(false); // a successful list dismisses a stale create-error
        }
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("useSessionSnapshots: snapshot.list failed", err);
        }
      });
  }, [sessionId]);

  // A session switch disables Resume immediately (safe default) — the list
  // itself lags until `refetch` (triggered below) resolves, same as the
  // original drawer.
  useEffect(() => {
    setHasHandoff(false);
  }, [sessionId]);

  useEffect(() => {
    refetch();
  }, [refetch]);

  useSnapshotCreated(sessionId, () => refetch());

  return { snapshots, hasHandoff, snapshotError, setSnapshotError, refetch };
}
