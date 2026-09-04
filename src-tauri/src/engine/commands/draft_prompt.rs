//! Prompt text for `commands::draft`. Const fragments + one builder so tests
//! can assert the catalogue is embedded. English only.

use serde_json::Value;

use super::draft::{Catalogue, DraftMode, CLAUDE_MODELS, CODEX_MODELS, COLOR_SWATCHES, LEVELS};

const TASK_AGENT: &str = "You are configuring ONE AI agent definition for Conclave, a macOS app that runs Claude Code / Codex agents as a team inside a project workspace. Draft the single best-fitting agent for the brief.";
const TASK_TEAM: &str = "You are staffing a TEAM of AI agents for Conclave, a macOS app that runs Claude Code / Codex agents as a team inside a project workspace. Draft the smallest team that covers the brief, with reporting lines.";
const RULES_COMMON: &str = "Rules:\n- Use ONLY ids that appear in the catalogue below (roles, skills, models, colours, levels). Never invent an id.\n- Prefer an existing agent definition (existingAgentDefId) when one already fits the job; then give no other fields for that entry.\n- Propose newRole only when no catalogue role fits; give it a concrete one-paragraph description written as standing instructions to the agent.\n- Names are short, distinctive, human-like (one word), unique within the draft and not already used in the catalogue.\n- rationale: one sentence. notes: one short paragraph of assumptions. Output English only.";
const RULES_TEAM: &str = "- Team shape: exactly one top-level lead (no supervisor) at level principal; every other agent has a supervisorKey; reviewers and researchers never supervise implementers; keep it to the fewest agents that cover the brief (max 12).\n- If a current roster is listed, EXTEND it: reuse those members via existingAgentDefId where sensible and do not duplicate their jobs.";
const LEVEL_MEANING: &str = "Levels: junior (executes well-specified tasks), mid (owns a task end to end), senior (owns a lane, reviews peers), principal (leads, rules on disputes).";

pub fn build_prompt(mode: DraftMode, brief: &str, cat: &Catalogue, schema: &Value) -> String {
    let mut p = String::new();
    p.push_str(match mode {
        DraftMode::Agent => TASK_AGENT,
        DraftMode::Team => TASK_TEAM,
    });
    p.push_str("\n\nReply with ONE JSON object matching this schema exactly:\n");
    p.push_str(&schema.to_string());
    p.push_str("\n\n");
    p.push_str(RULES_COMMON);
    if mode == DraftMode::Team {
        p.push('\n');
        p.push_str(RULES_TEAM);
    }
    p.push_str("\n\n## Catalogue\n\n### Roles (id — name: description; default skills)\n");
    for r in &cat.roles {
        p.push_str(&format!(
            "- {} — {}: {}; skills: {}\n",
            r.id,
            r.name,
            r.description.trim(),
            r.skill_ids.join(", ")
        ));
    }
    p.push_str("\n### Optional skills (id — name: description)\n");
    for s in &cat.skills {
        p.push_str(&format!(
            "- {} — {}: {}\n",
            s.id,
            s.name,
            s.description.as_deref().unwrap_or("").trim()
        ));
    }
    p.push_str(&format!(
        "\n### Models\n- claude-code: {}\n- codex: {}\n",
        CLAUDE_MODELS.join(", "),
        CODEX_MODELS.join(", ")
    ));
    p.push_str(&format!("\n### Colours\n{}\n", COLOR_SWATCHES.join(", ")));
    p.push_str(&format!(
        "\n### Levels\n{}\n{}\n",
        LEVELS.join(", "),
        LEVEL_MEANING
    ));
    p.push_str("\n### Existing agent definitions (id — name, role, cliKind/model)\n");
    if cat.existing.is_empty() {
        p.push_str("(none)\n");
    }
    for d in &cat.existing {
        p.push_str(&format!(
            "- {} — {}, {}, {}/{}\n",
            d.id,
            d.name,
            d.role_name.as_deref().unwrap_or("no role"),
            d.cli_kind.as_deref().unwrap_or("-"),
            d.model.as_deref().unwrap_or("-")
        ));
    }
    if mode == DraftMode::Team {
        p.push_str("\n### Current roster of this workspace (name — role, level, reports to)\n");
        if cat.roster.is_empty() {
            p.push_str("(empty)\n");
        }
        for m in &cat.roster {
            p.push_str(&format!(
                "- {} — {}, {}, reports to {}\n",
                m.name,
                m.role_name.as_deref().unwrap_or("no role"),
                m.level.as_deref().unwrap_or("-"),
                m.supervisor_name.as_deref().unwrap_or("nobody")
            ));
        }
    }
    p.push_str("\n## Brief\n\n```\n");
    p.push_str(brief.trim());
    p.push_str("\n```\n");
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::commands::draft::{draft_schema, tests::cat};

    #[test]
    fn prompt_embeds_catalogue_ids_brief_and_schema() {
        let c = cat();
        let schema = draft_schema(DraftMode::Team);
        let p = build_prompt(
            DraftMode::Team,
            "Port the billing service to Rust",
            &c,
            &schema,
        );
        for needle in [
            "implementer",
            "agent-loop",
            "def-existing",
            "claude-sonnet-5",
            "gpt-5.5",
            "Port the billing service to Rust",
            "\"agents\"",
        ] {
            assert!(p.contains(needle), "missing {needle}");
        }
        assert!(p.contains("exactly one top-level lead"));
        assert!(!build_prompt(DraftMode::Agent, "x", &c, &schema)
            .contains("exactly one top-level lead"));
    }

    #[test]
    fn team_mode_lists_the_roster_section_agent_mode_does_not() {
        let c = cat();
        let schema = draft_schema(DraftMode::Team);
        assert!(build_prompt(DraftMode::Team, "b", &c, &schema).contains("Current roster"));
        assert!(!build_prompt(DraftMode::Agent, "b", &c, &schema).contains("Current roster"));
    }

    /// Probe helper for the Task A5 manual gate: dumps the SHIPPED prompt and
    /// schema so a real `claude -p` run exercises exactly what production
    /// sends. Ignored by default and a no-op without `CONCLAVE_PROBE_OUT`, so
    /// it never writes during an ordinary `cargo test`.
    #[test]
    #[ignore]
    fn dump_prompt_for_probe() {
        let Ok(dir) = std::env::var("CONCLAVE_PROBE_OUT") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("probe dir");
        let schema = draft_schema(DraftMode::Team);
        let prompt = build_prompt(
            DraftMode::Team,
            "Port the billing service from Node to Rust with tests and a reviewer",
            &cat(),
            &schema,
        );
        std::fs::write(dir.join("prompt.txt"), &prompt).expect("write prompt");
        std::fs::write(dir.join("schema.json"), schema.to_string()).expect("write schema");
        eprintln!(
            "[probe] wrote {} bytes of prompt to {:?}",
            prompt.len(),
            dir
        );
    }
}
