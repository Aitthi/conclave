import clsx from "clsx";
import type { Tab } from "../lib/tabs";

// The page region — where the live native webview overlays in production. In the
// canon it's an intentional token-tinted skeleton of a page (never a bare fill),
// so the surrounding chrome reads. Loading shimmers; an ended session dims under
// a quiet marker.
export default function PageSurface({ tab }: { tab: Tab }) {
  const loading = tab.status === "loading";
  const ended = tab.status === "ended";

  return (
    <div className="relative min-h-0 flex-1 overflow-hidden bg-canvas">
      <div
        className={clsx(
          "mx-auto flex h-full max-w-3xl flex-col gap-6 px-10 py-9 transition-opacity",
          loading && "animate-pulse opacity-70",
          ended && "opacity-25",
        )}
      >
        {/* page masthead */}
        <div className="flex items-center gap-3">
          <div className="h-9 w-9 rounded-lg bg-fill" />
          <div className="flex flex-col gap-1.5">
            <div className="h-3 w-40 rounded bg-fill" />
            <div className="h-2.5 w-24 rounded bg-fill-soft" />
          </div>
          <div className="ml-auto flex gap-2">
            <div className="h-7 w-16 rounded-md bg-fill-soft" />
            <div className="h-7 w-20 rounded-md bg-fill" />
          </div>
        </div>

        {/* headline */}
        <div className="flex flex-col gap-2.5">
          <div className="h-7 w-2/3 rounded bg-fill" />
          <div className="h-3.5 w-full rounded bg-fill-soft" />
          <div className="h-3.5 w-11/12 rounded bg-fill-soft" />
          <div className="h-3.5 w-4/5 rounded bg-fill-soft" />
        </div>

        {/* media + aside */}
        <div className="grid grid-cols-3 gap-5">
          <div className="col-span-2 h-44 rounded-panel bg-fill" />
          <div className="flex flex-col gap-2.5 pt-1">
            <div className="h-3 w-full rounded bg-fill-soft" />
            <div className="h-3 w-5/6 rounded bg-fill-soft" />
            <div className="h-3 w-full rounded bg-fill-soft" />
            <div className="h-3 w-2/3 rounded bg-fill-soft" />
            <div className="mt-2 h-3 w-4/5 rounded bg-fill-soft" />
          </div>
        </div>

        {/* body continuation */}
        <div className="flex flex-col gap-2.5">
          <div className="h-3.5 w-full rounded bg-fill-soft" />
          <div className="h-3.5 w-full rounded bg-fill-soft" />
          <div className="h-3.5 w-10/12 rounded bg-fill-soft" />
          <div className="h-3.5 w-11/12 rounded bg-fill-soft" />
          <div className="h-3.5 w-3/4 rounded bg-fill-soft" />
        </div>

        {/* trailing cards row */}
        <div className="grid grid-cols-2 gap-5">
          <div className="h-24 rounded-panel bg-fill" />
          <div className="h-24 rounded-panel bg-fill" />
        </div>
      </div>

      {ended && (
        <div className="absolute inset-0 grid place-items-center">
          <div className="flex flex-col items-center gap-1 text-center">
            <span className="text-[14px] font-semibold text-text-secondary">Session ended</span>
            <span className="text-[12px] text-text-muted">
              {tab.owner.label} finished browsing · kept read-only for review
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
