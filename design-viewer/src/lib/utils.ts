// Tailwind-friendly class joiner (tiny clsx).
export function cn(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}

// hex (#rrggbb) → rgba string with alpha. Used for the inline-styled chrome
// where dynamic alpha tints can't be expressed as static Tailwind classes.
export function alpha(hex: string, al: number): string {
  try {
    const n = parseInt(String(hex).slice(1), 16);
    return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${al})`;
  } catch {
    return hex;
  }
}

export function nowLabel(): string {
  return new Date().toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
