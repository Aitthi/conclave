import { Component, type ReactNode } from "react";
import { MONO, useTheme } from "../../lib/theme";

// The AI writes .arta/state.json freely, so a tab can receive data in a shape the
// renderer didn't anticipate (e.g. a field that should be a string arriving as an
// object). React turns that into a render throw, which — without a boundary — takes
// down the WHOLE viewer (blank screen), not just the offending view. This boundary
// keeps the chrome (Topbar/Tabs/StatusBar) alive, shows a localized, themed message,
// reports the error so the agent can see it via arta_get_view, and recovers on its
// own the moment a new state arrives (resetKey changes).
export class ErrorBoundary extends Component<
  { children: ReactNode; resetKey?: unknown; onError?: (message: string) => void },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    this.props.onError?.(`viewer: ${error.message}`);
  }

  componentDidUpdate(prev: { resetKey?: unknown }) {
    // A fresh state push (new resetKey) might fix the bad data — clear and retry.
    if (this.state.error && prev.resetKey !== this.props.resetKey) {
      this.setState({ error: null });
    }
  }

  render() {
    if (this.state.error) return <Fallback message={this.state.error.message} />;
    return this.props.children;
  }
}

function Fallback({ message }: { message: string }) {
  const { c } = useTheme();
  return (
    <div className="flex flex-1 items-center justify-center p-8" style={{ background: c.bg }}>
      <div
        className="max-w-[420px] rounded-xl p-5 text-center"
        style={{ background: c.panel, border: `1px solid ${c.border2}` }}
      >
        <div
          className="mb-2 text-[10.5px] font-medium uppercase tracking-[0.8px]"
          style={{ fontFamily: MONO, color: c.red }}
        >
          Couldn't render this view
        </div>
        <div className="mb-3 text-[13px] leading-[1.5]" style={{ color: c.dim }}>
          The data in <span style={{ fontFamily: MONO, color: c.text }}>.arta/state.json</span> has an
          unexpected shape for this tab. The rest of the viewer is fine — switch tabs, or have the agent
          fix the section.
        </div>
        <div
          className="rounded-md px-2.5 py-1.5 text-left text-[11px] leading-[1.45]"
          style={{ fontFamily: MONO, background: c.bg, border: `1px solid ${c.border}`, color: c.faint }}
        >
          {message}
        </div>
      </div>
    </div>
  );
}
