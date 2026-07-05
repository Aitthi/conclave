import { Box, Flag } from "lucide-react";
import type { Milestone, Plan, PlanStatus, Task, TaskPriority } from "../../lib/types";
import { MONO, useTheme, type DarkTokens } from "../../lib/theme";

const COL_W = 268;

function defaultStatuses(c: DarkTokens): PlanStatus[] {
  return [
    { id: "todo", name: "To do", color: c.faint },
    { id: "doing", name: "In progress", color: c.amber },
    { id: "done", name: "Done", color: c.green },
  ];
}

function priorityColor(p: TaskPriority | undefined, c: DarkTokens): string | null {
  switch (p) {
    case "urgent": return c.red;
    case "high": return "#fb923c";
    case "normal": return c.accent;
    case "low": return c.faint;
    default: return null;
  }
}

function TaskCard({ task, c }: { task: Task; c: DarkTokens }) {
  const pri = priorityColor(task.priority, c);
  return (
    <div
      style={{
        background: c.card,
        border: `1px solid ${c.border2}`,
        borderRadius: 9,
        padding: "9px 11px",
        boxShadow: c.shadow,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 7 }}>
        {pri && <Flag size={12} color={pri} fill={pri} style={{ flexShrink: 0, marginTop: 2 }} />}
        <span style={{ fontSize: 13, color: c.text, lineHeight: 1.35 }}>{task.title}</span>
      </div>
      {task.priority && (
        <div style={{ marginTop: 6, fontFamily: MONO, fontSize: 9.5, letterSpacing: 0.5, textTransform: "uppercase", color: pri || c.faint }}>
          {task.priority}
        </div>
      )}
    </div>
  );
}

function Swimlane({ milestone, statuses, c }: { milestone: Milestone; statuses: PlanStatus[]; c: DarkTokens }) {
  const tasks = milestone.tasks || [];
  const doneIds = new Set(statuses.filter((s) => /done|complete|closed/i.test(s.id) || /done|complete|closed/i.test(s.name)).map((s) => s.id));
  const done = tasks.filter((t) => doneIds.has(t.status)).length;
  const pct = tasks.length ? Math.round((done / tasks.length) * 100) : 0;
  return (
    <div style={{ marginBottom: 22 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 10 }}>
        <span style={{ fontSize: 14, fontWeight: 600, color: c.text }}>{milestone.name}</span>
        <span style={{ fontFamily: MONO, fontSize: 11, color: c.dim }}>{done}/{tasks.length}</span>
        <div style={{ width: 120, height: 5, borderRadius: 3, overflow: "hidden", background: c.panel2 }}>
          <div style={{ width: `${pct}%`, height: "100%", background: c.accent, transition: "width .4s ease" }} />
        </div>
      </div>
      <div style={{ display: "flex", gap: 12 }}>
        {statuses.map((s) => {
          const cards = tasks.filter((t) => t.status === s.id);
          return (
            <div key={s.id} style={{ width: COL_W, flexShrink: 0, display: "flex", flexDirection: "column", gap: 8 }}>
              {cards.length === 0 ? (
                <div style={{ border: `1px dashed ${c.borderSoft}`, borderRadius: 8, padding: "10px 11px", fontFamily: MONO, fontSize: 11, color: c.faint }}>—</div>
              ) : (
                cards.map((t, i) => <TaskCard key={i} task={t} c={c} />)
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function PlanTab({ plan }: { plan: Plan }) {
  const { c } = useTheme();
  const statuses = plan.statuses && plan.statuses.length ? plan.statuses : defaultStatuses(c);
  const milestones = plan.milestones || [];
  const totalTasks = milestones.reduce((n, m) => n + (m.tasks?.length || 0), 0);

  return (
    <div className="flex h-full w-full flex-col">
      {/* header: title + counts */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "0 18px", minHeight: 46, borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0 }}>
        <span style={{ fontFamily: MONO, fontSize: 13, color: c.text }}>Plan</span>
        <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{milestones.length} milestones · {totalTasks} tasks</span>
      </div>

      {(plan.stack || []).length > 0 && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 18px", borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0, overflowX: "auto" }}>
          <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, fontFamily: MONO, fontSize: 10, fontWeight: 600, letterSpacing: 0.5, textTransform: "uppercase", color: c.faint }}>
            <Box size={12} /> Stack
          </span>
          {(plan.stack || []).map((s) => (
            <span key={s} style={{ flexShrink: 0, fontFamily: MONO, fontSize: 11, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 6, padding: "3px 8px", whiteSpace: "nowrap" }}>{s}</span>
          ))}
        </div>
      )}

      {/* board */}
      {totalTasks === 0 && milestones.length === 0 ? (
        <div className="flex flex-1 items-center justify-center">
          <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
            <Flag size={28} />
            <div className="max-w-[300px] text-[13px] leading-relaxed">No plan yet — milestones and tasks appear here as a Kanban board.</div>
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto" style={{ padding: "18px" }}>
          <div style={{ minWidth: statuses.length * (COL_W + 12) }}>
            {/* column headers */}
            <div className="sticky top-0 z-10" style={{ display: "flex", gap: 12, paddingBottom: 12, background: c.bg }}>
              {statuses.map((s) => {
                const col = s.color || c.dim;
                const count = milestones.reduce((n, m) => n + (m.tasks || []).filter((t) => t.status === s.id).length, 0);
                return (
                  <div key={s.id} style={{ width: COL_W, flexShrink: 0, display: "flex", alignItems: "center", gap: 8, padding: "6px 10px", borderRadius: 8, background: c.panel2, border: `1px solid ${c.border2}` }}>
                    <span style={{ width: 8, height: 8, borderRadius: 99, background: col }} />
                    <span style={{ fontFamily: MONO, fontSize: 12, fontWeight: 600, color: c.text, flex: 1 }}>{s.name}</span>
                    <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{count}</span>
                  </div>
                );
              })}
            </div>
            {milestones.map((m, i) => (
              <Swimlane key={i} milestone={m} statuses={statuses} c={c} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
