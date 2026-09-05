// src/components/builder/readiness.ts
//
// Per-section readiness for the agent Builder (spec D7). Pure: the shell
// passes a plain snapshot of its state, the rail and footer render the result.
// An empty model is NOT a blocker — it means "Auto (authenticated default)".

export type SectionId = "identity" | "role" | "runtime" | "skills" | "position";
export type Readiness = "complete" | "incomplete" | "error";
export type CliAvailabilityState =
  "idle" | "checking" | "available" | "missing" | "error";

export interface ReadinessInput {
  name: string;
  isAntigravity: boolean;
  cliAvailabilityState: CliAvailabilityState;
  isEditing: boolean;
}

export const SECTION_ORDER: SectionId[] = [
  "identity",
  "role",
  "runtime",
  "skills",
  "position",
];

export const SECTION_LABELS: Record<SectionId, string> = {
  identity: "Identity",
  role: "Role & Level",
  runtime: "Runtime",
  skills: "Skills",
  position: "Position",
};

function runtimeReadiness(input: ReadinessInput): Readiness {
  if (!input.isAntigravity) return "complete";
  switch (input.cliAvailabilityState) {
    case "available":
      return "complete";
    case "missing":
    case "error":
      return "error";
    default:
      return "incomplete";
  }
}

export function sectionReadiness(
  input: ReadinessInput,
): Record<SectionId, Readiness> {
  return {
    identity: input.name.trim().length > 0 ? "complete" : "incomplete",
    role: "complete",
    runtime: runtimeReadiness(input),
    skills: "complete",
    position: "complete",
  };
}

/** First reason the primary button is disabled, in display order; null = ready. */
export function firstBlocker(input: ReadinessInput): string | null {
  if (input.name.trim().length === 0) return "Name required";
  if (input.isAntigravity) {
    if (input.cliAvailabilityState === "missing")
      return "Install agy to continue";
    if (
      input.cliAvailabilityState === "idle" ||
      input.cliAvailabilityState === "checking"
    ) {
      return "Checking agy…";
    }
  }
  return null;
}

export function readyLabel(input: ReadinessInput): string {
  return input.isEditing ? "Ready to save" : "Ready to create";
}

/** True when the blocker should render in the danger colour. */
export function blockerIsDanger(blocker: string | null): boolean {
  return blocker === "Install agy to continue";
}
