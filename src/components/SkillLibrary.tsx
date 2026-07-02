import { useEffect, useState } from "react";
import { Wand2, Search, Plus, Pencil, Trash2, X } from "lucide-react";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";
import { SkillEditor } from "./SkillEditor";

export interface SkillLibraryProps {
  onClose: () => void;
}

interface CustomSkillCardProps {
  skill: Skill;
  onEdit: () => void;
  onDelete: () => void;
  deleting: boolean;
}

function CustomSkillCard({ skill, onEdit, onDelete, deleting }: CustomSkillCardProps) {
  const [confirming, setConfirming] = useState(false);
  const count = skill.attachedTo ?? 0;
  const countLabel = count === 0 ? "Not attached to any agent" : `attached to ${count} agent${count !== 1 ? "s" : ""}`;

  return (
    <div className="rounded-xl p-3.5 ring-hair bg-surface">
      <div className="flex items-start gap-3">
        <div className="w-10 h-10 rounded-[11px] bg-accent/[0.12] text-accent grid place-items-center shrink-0">
          <Wand2 className="w-5 h-5" />
        </div>
        <div className="flex-1 min-w-0">
          <span className="text-[13.5px] font-semibold">{skill.name}</span>
          <div className="text-[11px] text-text-muted truncate">{skill.description || "No description"}</div>
          <div className="text-[10.5px] text-text-muted mt-1">{countLabel}</div>
        </div>
      </div>
      <div className="flex items-center gap-1.5 mt-3">
        <button
          onClick={onEdit}
          className="flex-1 text-[11.5px] font-medium text-text-body bg-surface ring-hair rounded-lg py-1.5 hover:bg-overlay/[0.02] flex items-center justify-center gap-1"
        >
          <Pencil className="w-3.5 h-3.5" />
          Edit
        </button>
        {confirming ? (
          <button
            onClick={onDelete}
            disabled={deleting}
            onMouseLeave={() => setConfirming(false)}
            className="flex-1 text-[11.5px] font-semibold text-white bg-danger rounded-lg py-1.5 hover:brightness-105 disabled:opacity-50 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            {deleting ? "Deleting…" : "Confirm"}
          </button>
        ) : (
          <button
            onClick={() => setConfirming(true)}
            className="flex-1 text-[11.5px] font-medium text-danger bg-danger/[0.06] rounded-lg py-1.5 hover:bg-danger/10 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            Delete
          </button>
        )}
      </div>
    </div>
  );
}

function SystemSkillCard({ skill }: { skill: Skill }) {
  return (
    <div className="rounded-xl p-3.5 ring-hair bg-surface opacity-80">
      <div className="flex items-start gap-3">
        <div className="w-10 h-10 rounded-[11px] bg-overlay/[0.06] text-text-secondary grid place-items-center shrink-0">
          <Wand2 className="w-5 h-5" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13.5px] font-semibold">{skill.name}</span>
            <span
              className={
                skill.mandatory
                  ? "text-[9.5px] font-medium text-text-muted bg-overlay/[0.05] px-1.5 py-px rounded"
                  : "text-[9.5px] font-medium text-accent bg-accent/[0.12] px-1.5 py-px rounded"
              }
            >
              {skill.mandatory ? "Always on" : "Optional"}
            </span>
          </div>
          <div className="text-[11px] text-text-muted truncate">{skill.description || "No description"}</div>
        </div>
      </div>
    </div>
  );
}

export function SkillLibrary({ onClose }: SkillLibraryProps) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [editingSkill, setEditingSkill] = useState<Skill | undefined>(undefined);
  const [showEditor, setShowEditor] = useState(false);

  async function loadSkills() {
    try {
      setSkills(await ipc.skill.list());
      setLoadError(false);
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("SkillLibrary: skill.list failed", err);
      setSkills([]);
      setLoadError(true);
    }
  }

  useEffect(() => {
    loadSkills();
  }, []);

  async function handleDelete(id: string) {
    setDeletingId(id);
    try {
      await ipc.skill.delete({ id });
      await loadSkills();
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("SkillLibrary: skill.delete failed", err);
    } finally {
      setDeletingId(null);
    }
  }

  const q = search.trim().toLowerCase();
  const matches = (s: Skill) =>
    !q || s.name.toLowerCase().includes(q) || (s.description ?? "").toLowerCase().includes(q);
  const systemSkills = skills.filter((s) => s.kind === "builtin" && matches(s));
  const customSkills = skills.filter((s) => s.kind === "custom" && matches(s));

  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />

      <div className="relative w-[440px] max-w-full h-full bg-sidebar shadow-2xl flex flex-col ring-1 ring-overlay/[0.08]">
        <div className="h-12 flex items-center gap-2 px-4 border-b border-overlay/[0.06] shrink-0">
          <Wand2 className="w-[15px] h-[15px] text-accent shrink-0" />
          <span className="text-[13px] font-semibold tracking-tight">Skill Library</span>
          <button
            onClick={onClose}
            className="ml-auto w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted shrink-0"
            aria-label="Close Skill Library"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="px-3 pt-3 pb-2 shrink-0">
          <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7">
            <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search skills"
              className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
            />
          </div>
        </div>

        <div className="flex-1 overflow-y-auto scroll-thin px-3 pb-3 space-y-4">
          {loadError ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-center px-6">
              <Wand2 className="w-9 h-9 text-text-quaternary" />
              <p className="text-[13px] font-semibold text-text-secondary">Failed to load skills</p>
              <p className="text-[11.5px] text-text-tertiary">Check the app is running and try again</p>
            </div>
          ) : (
            <>
              {systemSkills.length > 0 && (
                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                    System
                  </div>
                  <div className="space-y-2">
                    {systemSkills.map((s) => (
                      <SystemSkillCard key={s.id} skill={s} />
                    ))}
                  </div>
                </div>
              )}
              <div>
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                  Custom
                </div>
                {customSkills.length === 0 ? (
                  <p className="text-[11.5px] text-text-tertiary px-0.5">
                    {skills.length === 0 ? "No skills yet" : "No matching custom skills"}
                  </p>
                ) : (
                  <div className="space-y-2">
                    {customSkills.map((s) => (
                      <CustomSkillCard
                        key={s.id}
                        skill={s}
                        onEdit={() => {
                          setEditingSkill(s);
                          setShowEditor(true);
                        }}
                        onDelete={() => handleDelete(s.id)}
                        deleting={deletingId === s.id}
                      />
                    ))}
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        <div className="border-t border-overlay/[0.06] p-2 shrink-0">
          <button
            onClick={() => {
              setEditingSkill(undefined);
              setShowEditor(true);
            }}
            className="w-full flex items-center justify-center gap-1.5 px-2 py-2 rounded-lg bg-accent text-white hover:brightness-105"
          >
            <Plus className="w-4 h-4" />
            <span className="text-[12.5px] font-semibold">New skill</span>
          </button>
        </div>
      </div>

      {showEditor && (
        <SkillEditor
          initialSkill={editingSkill}
          onClose={() => setShowEditor(false)}
          onSaved={() => {
            setShowEditor(false);
            loadSkills();
          }}
        />
      )}
    </div>
  );
}
