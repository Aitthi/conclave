import { useState } from "react";
import { X, Wand2 } from "lucide-react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";

export interface SkillEditorProps {
  onClose: () => void;
  onSaved: (skill: Skill) => void;
  /** Pre-fill the form for editing an existing CUSTOM skill. Never a builtin
   *  one — the Library never opens the editor for builtin cards. */
  initialSkill?: Skill;
}

/**
 * Create or edit a CUSTOM skill: name, short description (shown in Library
 * lists), and the full markdown `content` injected into a cli agent's skill
 * sidecar file at launch (see docs/adr/0001-skill-system-v1.md). Builtin
 * skills are never edited here. Full-panel (not a small modal) so there's
 * room for a real code editor and, alongside it, an agent-assist panel (see
 * docs/specs/2026-07-02-skill-editor-agent-assist-design.md).
 */
export function SkillEditor({ onClose, onSaved, initialSkill }: SkillEditorProps) {
  const isEditing = initialSkill !== undefined;
  const [name, setName] = useState(initialSkill?.name ?? "");
  const [description, setDescription] = useState(initialSkill?.description ?? "");
  const [content, setContent] = useState(initialSkill?.content ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSave() {
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const skill = await ipc.skill.save({
        id: initialSkill?.id,
        name: name.trim(),
        description: description.trim() || undefined,
        content,
      });
      onSaved(skill);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-surface">
      <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
        <div className="flex items-center gap-2">
          <Wand2 className="w-4 h-4 text-accent" />
          <span className="text-[13px] font-semibold tracking-tight">
            {isEditing ? "Edit skill" : "New skill"}
          </span>
        </div>
        <button
          onClick={onClose}
          disabled={saving}
          className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-50"
          aria-label="Close"
        >
          <X className="w-[15px] h-[15px]" />
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        <div className="flex-1 flex flex-col min-w-0 p-5 overflow-y-auto scroll-thin">
          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Name
            </div>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Code Reviewer"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Description
            </div>
            <input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Shown in the Skill Library list"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="flex-1 min-h-0 flex flex-col">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Content
            </div>
            <div className="flex-1 min-h-0 rounded-lg ring-1 ring-overlay/[0.10] overflow-hidden">
              <CodeMirror
                value={content}
                onChange={(value) => setContent(value)}
                extensions={[markdown()]}
                height="100%"
                className="h-full text-[12.5px]"
              />
            </div>
          </div>

          {error && <p className="text-[12px] text-danger mt-3">{error}</p>}
        </div>
      </div>

      <div className="border-t border-overlay/[0.07] px-5 py-3 bg-surface shrink-0 flex items-center gap-2">
        <button
          onClick={onClose}
          disabled={saving}
          className="flex-1 text-[12.5px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-2.5 hover:bg-overlay/[0.02] disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save skill"}
        </button>
      </div>
    </div>
  );
}
