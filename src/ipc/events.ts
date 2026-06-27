import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Known event names
// ---------------------------------------------------------------------------

export const EVENT_NAMES = {
  sessionOutput: "session:output",
  sessionStatus: "session:status",
  fusionStage: "fusion:stage",
  messageInjected: "message:injected",
} as const;

export type EventName = (typeof EVENT_NAMES)[keyof typeof EVENT_NAMES];

// ---------------------------------------------------------------------------
// Event payload types
// ---------------------------------------------------------------------------

export interface SessionOutputEvent {
  sessionId: string;
  chunk: string;
}

export interface SessionStatusEvent {
  sessionId: string;
  status: "running" | "idle" | "waiting";
}

export interface FusionStageEvent {
  runId: string;
  stage: "panel" | "judge" | "synthesize";
  data?: unknown;
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
 * Subscribe to all fusion stage events (not filtered by runId because a
 * single fusion run covers all stages; callers can filter if needed).
 */
export function useFusionStage(cb: (event: FusionStageEvent) => void): void {
  useEvent<FusionStageEvent>(EVENT_NAMES.fusionStage, cb);
}
