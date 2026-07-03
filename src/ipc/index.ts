// Public surface of the IPC layer.
// Import from "@/ipc" (or "src/ipc") in React components.

export type {
  Workspace,
  Provider,
  AgentDefinition,
  WorkspaceAgent,
  Session,
  Message,
  InterAgentMessage,
  BlackboardEntry,
  BlackboardActivity,
  Snapshot,
  Artifact,
  FusionRun,
  FusionPanelResponse,
  Tool,
  Skill,
  Role,
} from "./types";

export type { Commands } from "./commands";
export { call, ipc } from "./commands";

export type {
  EventName,
  SessionOutputEvent,
  SessionStatusEvent,
  SessionContextEvent,
  FusionStageEvent,
  MessageInjectedEvent,
  SnapshotCreatedEvent,
} from "./events";
export {
  EVENT_NAMES,
  useEvent,
  useSessionOutput,
  useSessionStatus,
  useSessionContext,
  useFusionStage,
  useMessageInjected,
  useAnyMessageInjected,
  useSnapshotCreated,
} from "./events";
