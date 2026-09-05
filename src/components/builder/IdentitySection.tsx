// src/components/builder/IdentitySection.tsx
//
// Identity: avatar + colour popover + name field + role/level line, and the
// "Drafted by" chip in the heading right slot. JSX moved verbatim from
// Builder.tsx (spec D9); the role line follows canon rule 11.

import type { CSSProperties, Dispatch, SetStateAction } from "react";
import { Sparkles } from "lucide-react";
import { COLOR_SWATCHES } from "../../lib/modelCatalogue";
import { Section } from "./Section";

interface IdentitySectionProps {
  name: string;
  setName: (v: string) => void;
  color: string;
  setColor: (v: string) => void;
  showColors: boolean;
  setShowColors: Dispatch<SetStateAction<boolean>>;
  letter: string;
  /** "Role \u00b7 Level" line under the name (canon rule 11). */
  identityLine: string;
  draftedBy?: string;
  touched: boolean;
  setTouched: (v: boolean) => void;
}

export function IdentitySection({
  name,
  setName,
  color,
  setColor,
  showColors,
  setShowColors,
  letter,
  identityLine,
  draftedBy,
  touched,
  setTouched,
}: IdentitySectionProps) {
  return (
    <Section
      id="identity"
      title="Identity"
      first
      actions={
        draftedBy && !touched ? (
          <span className="text-[11px] text-text-tertiary inline-flex items-center gap-1">
            <Sparkles className="w-3 h-3" />
            Drafted by {draftedBy}
          </span>
        ) : undefined
      }
    >
      <div className="flex items-center gap-2.5">
        {/* Avatar doubles as the color picker — click to choose. */}
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={() => setShowColors((v) => !v)}
            className="w-10 h-10 rounded-[10px] text-white grid place-items-center text-[15px] font-bold ring-1 ring-overlay/[0.06] hover:brightness-105"
            style={{ backgroundColor: color }}
            title="Change color"
            aria-label="Change color"
          >
            {letter}
          </button>
          {showColors && (
            <>
              {/* Click-away backdrop. */}
              <div className="fixed inset-0 z-10" onClick={() => setShowColors(false)} />
              <div className="absolute z-20 top-full left-0 mt-1.5 flex items-center gap-1.5 bg-surface rounded-xl ring-1 ring-overlay/[0.1] shadow-lg p-2">
                {COLOR_SWATCHES.map((swatch) => (
                  <button
                    key={swatch}
                    onClick={() => {
                      setColor(swatch);
                      setShowColors(false);
                    }}
                    className={`w-[18px] h-[18px] rounded-full transition-all ${
                      color === swatch ? "ring-2 ring-offset-1" : "hover:scale-110"
                    }`}
                    style={
                      {
                        backgroundColor: swatch,
                        "--tw-ring-color": swatch,
                      } as CSSProperties
                    }
                    aria-label={`Color ${swatch}`}
                  />
                ))}
                {/* Custom color — opens the OS color picker. The popover
                    stays open so the avatar preview updates live. */}
                <label
                  className="w-[18px] h-[18px] rounded-full cursor-pointer ring-1 ring-overlay/15 relative overflow-hidden shrink-0"
                  title="Custom color"
                  style={{
                    background:
                      "conic-gradient(red, yellow, lime, aqua, blue, magenta, red)",
                  }}
                >
                  <input
                    type="color"
                    value={color}
                    onChange={(e) => setColor(e.target.value)}
                    className="absolute inset-0 opacity-0 cursor-pointer"
                    aria-label="Custom color"
                  />
                </label>
              </div>
            </>
          )}
        </div>
        <div className="flex-1 space-y-1">
          <input
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setTouched(true);
            }}
            placeholder="Agent name"
            className="w-full text-[14px] font-semibold tracking-tight bg-transparent outline-none border-b border-overlay/10 focus:border-accent pb-0.5"
          />
          <div className="text-[11.5px] text-text-muted truncate">
            {identityLine}
          </div>
        </div>
      </div>
    </Section>
  );
}
