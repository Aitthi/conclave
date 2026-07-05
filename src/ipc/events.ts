import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Known event names
// ---------------------------------------------------------------------------

export const EVENT_NAMES = {
  sessionOutput: "session:output",
  sessionStatus: "session:status",
  sessionContext: "session:context",
  fusionStage: "fusion:stage",
  messageInjected: "message:injected",
  snapshotCreated: "snapshot:created",
  taskChanged: "task:changed",
  memoryChanged: "memory:changed",
  artifactChanged: "artifact:changed",
  rosterChanged: "roster:changed",
} as const;

export type EventName = (typeof EVENT_NAMES)[keyof typeof EVENT_NAMES];

// ---------------------------------------------------------------------------
// Event payload types
// ---------------------------------------------------------------------------

/**
 * `activity` (bb plan:working-false-positive) is `false` when the engine
 * judged this chunk to be the terminal's own echo of OUR input (a
 * resize-provoked repaint, keystroke echo) rather than genuine agent output.
 * Terminal still renders every chunk regardless — only Roster's working
 * indicator should ignore non-activity chunks.
 */
export interface SessionOutputEvent {
  sessionId: string;
  chunk: string;
  activity: boolean;
}

export interface SessionStatusEvent {
  sessionId: string;
  status: "running" | "idle" | "waiting";
}

/**
 * A live context-meter update for a session. HONESTY: `estimated` is always
 * `true` for now — `contextTokens` is derived from streamed output bytes
 * (≈4 chars/token), NOT real provider token-usage telemetry. The UI labels the
 * meter "estimate" accordingly.
 */
export interface SessionContextEvent {
  sessionId: string;
  contextTokens: number;
  contextLimit: number;
  estimated: boolean;
}

/**
 * A snapshot (manual or auto-compact) was persisted for a session. `tokens` and
 * `triggerPct` are the estimate at capture time (absent when the session had no
 * estimate yet).
 */
export interface SnapshotCreatedEvent {
  sessionId: string;
  snapshotId: string;
  // Named `type` (not `kind`) to align with the `Snapshot.type` domain field and
  // the `snapshot.create` payload — same union, one name across the wire.
  type: "auto" | "manual" | "handoff";
  tokens?: number;
  triggerPct?: number;
}

export interface FusionStageEvent {
  runId: string;
  stage: "panel" | "judge" | "synthesize";
  data?: unknown;
}

/**
 * A task's identity or lifecycle changed (created, claimed, state moved, event
 * logged) — ADR 0008 §Frozen bus. Emitted by every mutating `task.*` handler
 * (Lane A emits; Lane B reuses). Carries just enough to let the Lane Board
 * refetch the affected task without a payload diff: `workspaceId` scopes it,
 * `slug`/`state` let the board update a row optimistically. camelCase serde,
 * kept in sync with `engine/bus.rs` `TaskChanged` (serialisation tests enforce).
 */
export interface TaskChangedEvent {
  workspaceId: string;
  taskId: string;
  slug: string;
  state:
    | "planned"
    | "claimed"
    | "in_progress"
    | "review"
    | "merged"
    | "abandoned";
}

/**
 * A workspace's `memory_chunk` table actually changed — emitted after
 * `remember`/`delete`/`clear`/`approve` write or remove a chunk (NOT for a
 * no-op: a deduped remember/approve, a delete that found nothing, an empty
 * clear). Carries only `workspaceId` — the Memory Graph refetches rather than
 * patching state from the payload. camelCase serde, kept in sync with
 * `engine/bus.rs` `MemoryChanged` (serialisation tests enforce).
 */
export interface MemoryChangedEvent {
  workspaceId: string;
}

/**
 * A workspace's `artifact` table gained a row — emitted after `artifact.add`
 * (plan design-artifact-store). Carries only `workspaceId`; the Artifacts view
 * re-lists rather than patching state from the payload. camelCase serde, kept
 * in sync with `engine/bus.rs` `ArtifactChanged` (serialisation tests enforce).
 */
export interface ArtifactChangedEvent {
  workspaceId: string;
}

/**
 * A workspace's roster membership/position changed — emitted after
 * `instance.setPosition` writes a level or supervisor link (spec position-system
 * §5.1). Carries only `workspaceId`; an open Roster/org view refetches rather
 * than patching state from the payload. camelCase serde, kept in sync with
 * `engine/bus.rs` `RosterChanged` (serialisation tests enforce).
 */
export interface RosterChangedEvent {
  workspaceId: string;
}

/**
 * An inter-agent injection delivered to a target instance's live input.
 * Emitted only when the injection is actually delivered (target is live), so
 * `toSessionId` is normally present. The origin is carried as `fromInstanceId`;
 * the UI renders the "injected from X" chrome from this event.
 */
export interface MessageInjectedEvent {
  toInstanceId: string;
  toSessionId?: string;
  fromInstanceId: string;
  text: string;
  autoSubmitted: boolean;
}

// ---------------------------------------------------------------------------
// Generic useEvent hook
//
// Subscribes via Tauri's listen() in a useEffect and unsubscribes on cleanup.
// Guards against the component unmounting before the async listen() resolves.
// ---------------------------------------------------------------------------

export function useEvent<T>(
  event: string,
  handler: (payload: T) => void,
): void {
  // Keep a stable ref to the handler so callers don't need to memoize it.
  const handlerRef = useRef(handler);
  useEffect(() => {
    handlerRef.current = handler;
  });

  useEffect(() => {
    let active = true;
    let unlistenFn: UnlistenFn | undefined;

    // Fixture mode (DEV-only): subscribe to the local bus instead of the Tauri
    // host. Checked synchronously (not via the fixtures/mode dynamic import) so
    // this branch is exclusive with the listen() branch below — a dynamic-import
    // check would race listen() and double-subscribe. Mirrors fixtureScenario()
    // semantics (a truthy `?fixture=` value); prod short-circuits on DEV so the
    // whole branch dead-code-eliminates.
    const inFixture =
      import.meta.env.DEV &&
      !!new URLSearchParams(window.location.search).get("fixture");

    if (inFixture) {
      void import("../fixtures/events").then(({ fixtureListen }) => {
        if (!active) return;
        unlistenFn = fixtureListen<T>(event, (p) => {
          if (active) handlerRef.current(p);
        });
      });
    } else {
      listen<T>(event, (e) => {
        if (active) handlerRef.current(e.payload);
      })
        .then((unlisten) => {
          if (active) {
            unlistenFn = unlisten;
          } else {
            // Component unmounted before listen resolved — tear down immediately.
            unlisten();
          }
        })
        .catch((err) => {
          // Subscribe failures are expected in non-Tauri contexts (e.g. tests,
          // plain `vite` without the shell). In dev, surface them — a silent
          // failure here makes the component permanently deaf to events.
          if (import.meta.env.DEV) {
            console.error(`useEvent: failed to subscribe to "${event}"`, err);
          }
        });
    }

    return () => {
      active = false;
      unlistenFn?.();
    };
  }, [event]);
}

// ---------------------------------------------------------------------------
// Typed convenience hooks
// ---------------------------------------------------------------------------

/**
 * Subscribe to streamed output chunks for a specific session.
 * The callback is called with each `SessionOutputEvent` whose sessionId
 * matches the one provided.
 */
export function useSessionOutput(
  sessionId: string,
  cb: (event: SessionOutputEvent) => void,
): void {
  useEvent<SessionOutputEvent>(EVENT_NAMES.sessionOutput, (payload) => {
    if (payload.sessionId === sessionId) cb(payload);
  });
}

/**
 * Subscribe to status changes for a specific session.
 * The callback is called with each `SessionStatusEvent` whose sessionId
 * matches the one provided.
 */
export function useSessionStatus(
  sessionId: string,
  cb: (event: SessionStatusEvent) => void,
): void {
  useEvent<SessionStatusEvent>(EVENT_NAMES.sessionStatus, (payload) => {
    if (payload.sessionId === sessionId) cb(payload);
  });
}

/**
 * Subscribe to live context-meter updates for a specific session.
 * The callback fires for each `SessionContextEvent` whose sessionId matches.
 */
export function useSessionContext(
  sessionId: string,
  cb: (event: SessionContextEvent) => void,
): void {
  useEvent<SessionContextEvent>(EVENT_NAMES.sessionContext, (payload) => {
    if (payload.sessionId === sessionId) cb(payload);
  });
}

/**
 * Subscribe to snapshot-created events for a specific session (manual or
 * auto-compact). The callback fires for each `SnapshotCreatedEvent` whose
 * sessionId matches — drives the snapshot list to refresh live.
 */
export function useSnapshotCreated(
  sessionId: string,
  cb: (event: SnapshotCreatedEvent) => void,
): void {
  useEvent<SnapshotCreatedEvent>(EVENT_NAMES.snapshotCreated, (payload) => {
    if (payload.sessionId === sessionId) cb(payload);
  });
}

/**
 * Subscribe to inter-agent injections involving a specific instance.
 * The callback fires for each `MessageInjectedEvent` where the instance is the
 * recipient (`toInstanceId`) OR the sender (`fromInstanceId`) — so a single
 * subscription drives both the receiver's inbox and the sender's outbox.
 */
export function useMessageInjected(
  instanceId: string,
  cb: (event: MessageInjectedEvent) => void,
): void {
  useEvent<MessageInjectedEvent>(EVENT_NAMES.messageInjected, (payload) => {
    if (payload.toInstanceId === instanceId || payload.fromInstanceId === instanceId) cb(payload);
  });
}

/**
 * Subscribe to EVERY inter-agent injection, unfiltered — the Chat Hub shows a
 * whole workspace's traffic, so it refetches on any injection rather than
 * filtering per instance (its query is workspace-scoped server-side; a
 * cross-workspace event costs one cheap, seq-guarded refetch).
 */
export function useAnyMessageInjected(cb: (event: MessageInjectedEvent) => void): void {
  useEvent<MessageInjectedEvent>(EVENT_NAMES.messageInjected, cb);
}

/**
 * Subscribe to all fusion stage events (not filtered by runId because a
 * single fusion run covers all stages; callers can filter if needed).
 */
export function useFusionStage(cb: (event: FusionStageEvent) => void): void {
  useEvent<FusionStageEvent>(EVENT_NAMES.fusionStage, cb);
}

/**
 * Subscribe to every `task:changed` event, unfiltered — the Lane Board shows a
 * whole workspace's tasks, so it refetches on any change rather than
 * filtering per task (mirrors `useAnyMessageInjected`'s Chat Hub rationale).
 */
export function useAnyTaskChanged(cb: (event: TaskChangedEvent) => void): void {
  useEvent<TaskChangedEvent>(EVENT_NAMES.taskChanged, cb);
}

/**
 * Subscribe to task-changed events for a specific workspace — the Lane Board's
 * live-refresh trigger. The callback fires for each `TaskChangedEvent` whose
 * `workspaceId` matches, so a single subscription keeps one workspace's board
 * current across every agent's `task.*` mutation.
 */
export function useTaskChanged(
  workspaceId: string,
  cb: (event: TaskChangedEvent) => void,
): void {
  useEvent<TaskChangedEvent>(EVENT_NAMES.taskChanged, (payload) => {
    if (payload.workspaceId === workspaceId) cb(payload);
  });
}

/**
 * Subscribe to memory-changed events for a specific workspace — the Memory
 * Graph's live-refresh trigger (mirrors `useTaskChanged`'s Lane Board
 * rationale). Fires for each `MemoryChangedEvent` whose `workspaceId`
 * matches, so a `conclave memory remember` from any agent/terminal refreshes
 * an open graph view without reopening it.
 */
export function useMemoryChanged(
  workspaceId: string,
  cb: (event: MemoryChangedEvent) => void,
): void {
  useEvent<MemoryChangedEvent>(EVENT_NAMES.memoryChanged, (payload) => {
    if (payload.workspaceId === workspaceId) cb(payload);
  });
}

/**
 * Subscribe to artifact-changed events for a specific workspace — the Artifacts
 * view's live-refresh trigger (mirrors `useMemoryChanged`). Fires for each
 * `ArtifactChangedEvent` whose `workspaceId` matches, so a `conclave artifact
 * add` from any agent refreshes an open Artifacts view without reopening it.
 */
export function useArtifactChanged(
  workspaceId: string,
  cb: (event: ArtifactChangedEvent) => void,
): void {
  useEvent<ArtifactChangedEvent>(EVENT_NAMES.artifactChanged, (payload) => {
    if (payload.workspaceId === workspaceId) cb(payload);
  });
}

/**
 * Subscribe to roster-changed events for a specific workspace — the Roster/org
 * view's live-refresh trigger (mirrors `useArtifactChanged`). Fires for each
 * `RosterChangedEvent` whose `workspaceId` matches, so a `conclave position set`
 * from any agent refreshes an open roster without reopening it.
 */
export function useRosterChanged(
  workspaceId: string,
  cb: (event: RosterChangedEvent) => void,
): void {
  useEvent<RosterChangedEvent>(EVENT_NAMES.rosterChanged, (payload) => {
    if (payload.workspaceId === workspaceId) cb(payload);
  });
}
