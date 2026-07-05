import type { LucideIcon } from "lucide-react";
import { MONO, useTheme } from "../../lib/theme";
import { alpha } from "../../lib/utils";

export function Card({ children, className }: { children: React.ReactNode; className?: string }) {
  const { c } = useTheme();
  return (
    <div
      className={className}
      style={{ background: c.card, border: `1px solid ${c.border}`, borderRadius: 12, padding: "18px 20px" }}
    >
      {children}
    </div>
  );
}

export function SectionHead({ label, Icon }: { label: string; Icon?: LucideIcon }) {
  const { c } = useTheme();
  return (
    <div className="mb-3.5 flex items-center gap-2">
      {Icon && <Icon size={14} color={c.faint} />}
      <span
        className="text-[11px] font-medium uppercase tracking-[0.6px]"
        style={{ fontFamily: MONO, color: c.faint }}
      >
        {label}
      </span>
    </div>
  );
}

export function Tag({ label, color }: { label: string; color: string }) {
  return (
    <span
      className="rounded-[5px] px-1.5 py-px text-[9.5px] font-semibold tracking-[0.4px]"
      style={{ fontFamily: MONO, color, border: `1px solid ${alpha(color, 0.4)}`, background: alpha(color, 0.12) }}
    >
      {label}
    </span>
  );
}
