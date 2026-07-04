export type TermTabMode = "remount" | "keep-alive";

// The one-line revert point. Flip this back to "keep-alive" (and rebuild) to
// restore the pre-remount behavior where every tab's xterm stays mounted and
// only its visibility is toggled.
//
//   "remount"    — only the ACTIVE tab's <Terminal> is mounted; inactive tabs
//                  unmount. On the next mount the pre-remount buffer is restored
//                  from a serialize-addon snapshot, separated from the live TUI
//                  frame by a dim divider. (NEW DEFAULT)
//   "keep-alive" — every tab's xterm stays mounted; inactive ones are hidden via
//                  the `hidden` class. Verbatim pre-remount behavior.
const DEFAULT_MODE: TermTabMode = "remount";

// A dev-only override so a mode flip needs a reload, not a rebuild:
//   localStorage.setItem("conclave.termTabMode", "keep-alive"); location.reload();
// Deliberately NOT surfaced in any user-facing UI (human ruling: not a user
// concern). Read ONCE at module load — the mode cannot change within a page
// lifetime, so there is no setter, no subscription, no live switching.
const STORAGE_KEY = "conclave.termTabMode";

function readMode(): TermTabMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "remount" || v === "keep-alive") return v;
  } catch {
    // localStorage can throw in locked-down contexts — fall through to default.
  }
  return DEFAULT_MODE;
}

const MODE: TermTabMode = readMode();

export function getTermTabMode(): TermTabMode {
  return MODE;
}
