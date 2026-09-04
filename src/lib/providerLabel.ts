// Which provider/model an agent runs on, rendered as one short line of text.
//
// Human request 2026-09-04: a Roster row never said whether an agent was a
// Claude Code or a Codex process. The chip replaces the generic `>_` icon on
// the name line, so the indicator is TEXT, never colour alone (PRODUCT.md
// accessibility). Every function here is pure and total — a roster row with
// half-filled data returns null, it never throws.

/** The shape both a `WorkspaceAgent` (enriched by `instance.list`) and an
 *  `AgentDefinition` satisfy — callers pass whichever they hold. */
export interface ProviderSource {
  type?: string;
  cliKind?: string | null;
  providerId?: string | null;
}

/** "Claude" / "Codex" / "CLI" / "Chat · Anthropic" …, or null when the agent
 *  runs on nothing nameable (orchestrator, or a type we don't know). */
export function providerLabel(a: ProviderSource): string | null {
  if (a.type === "cli") {
    switch (a.cliKind) {
      case "claude-code":
        return "Claude";
      case "codex":
        return "Codex";
      case "custom":
        return "CLI";
      default:
        return "CLI";
    }
  }
  if (a.type === "chat") {
    switch (a.providerId) {
      case "anthropic":
        return "Chat · Anthropic";
      case "openai":
        return "Chat · OpenAI";
      case "local":
        return "Chat · Local";
      default:
        return "Chat";
    }
  }
  return null;
}

/**
 * The model id shortened for a chip: drop the `claude-` vendor prefix (the
 * chip already says "Claude") and a trailing release date.
 *
 *   claude-haiku-4-5-20251001 → haiku-4-5
 *   claude-opus-5             → opus-5
 *   gpt-5.6-sol               → gpt-5.6-sol
 *
 * The full id stays available for the `title` attribute.
 */
export function shortModel(model?: string | null): string | null {
  const trimmed = model?.trim();
  if (!trimmed) return null;
  const short = trimmed.replace(/^claude-/, "").replace(/-\d{8}$/, "");
  return short || trimmed;
}

/** The chip text: provider and model when both are known, otherwise whichever
 *  one is — null when neither. */
export function providerChip(
  a: ProviderSource & { model?: string | null },
): string | null {
  const label = providerLabel(a);
  const model = shortModel(a.model);
  if (label && model) return `${label} · ${model}`;
  return label ?? model;
}
