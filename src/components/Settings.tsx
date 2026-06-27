import { useEffect, useState } from "react";
import { X, Key } from "lucide-react";
import { ipc } from "../ipc";
import type { Provider } from "../ipc";

// ── Types ────────────────────────────────────────────────────────────────────

interface SettingsProps {
  onClose: () => void;
}

type ProviderName = Provider["name"];

interface ProviderEntry {
  name: ProviderName;
  label: string;
}

// ── Constants ────────────────────────────────────────────────────────────────

// "custom" is intentionally omitted: the chat runtime has no custom-endpoint
// provider arm yet, so offering it here would let a key be saved that the
// runtime can never use. Deferred until that arm exists.
const PROVIDERS: ProviderEntry[] = [
  { name: "anthropic", label: "Anthropic" },
  { name: "openai", label: "OpenAI" },
  { name: "local", label: "Local (Ollama / LM Studio)" },
];

// ── Sub-component: a single provider row ─────────────────────────────────────

interface ProviderRowProps {
  entry: ProviderEntry;
  saved: Provider | undefined;
  onSaved: () => void;
}

function ProviderCard({ entry, saved, onSaved }: ProviderRowProps) {
  const [keyInput, setKeyInput] = useState("");
  const [baseUrlInput, setBaseUrlInput] = useState(saved?.baseUrl ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Sync baseUrl field when the saved value changes (e.g. after list refetch).
  useEffect(() => {
    setBaseUrlInput(saved?.baseUrl ?? "");
  }, [saved?.baseUrl]);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await ipc.provider.upsert({
        name: entry.name,
        key: keyInput.trim() || undefined,
        baseUrl: baseUrlInput.trim() || undefined,
      });
      setKeyInput(""); // clear key field after save — never keep a raw key in state
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  const hasKey = Boolean(saved?.maskedKey);

  return (
    <div className="border border-black/[0.07] rounded-xl p-4 space-y-3">
      {/* Provider label + key status */}
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-semibold text-[#1d1d1f]">{entry.label}</span>
        <span
          className={`text-[11px] font-medium px-2 py-0.5 rounded-full ${
            hasKey
              ? "bg-[#e8f8ef] text-[#1c7c3f]"
              : "bg-black/[0.04] text-[#86868b]"
          }`}
        >
          {hasKey ? "Key saved" : "No key"}
        </span>
      </div>

      {/* Base URL (shown for all; required for local/custom) */}
      <div className="space-y-1">
        <label className="text-[11px] text-[#6e6e73] font-medium uppercase tracking-wide">
          Base URL
        </label>
        <input
          type="url"
          value={baseUrlInput}
          onChange={(e) => setBaseUrlInput(e.target.value)}
          placeholder={
            entry.name === "anthropic"
              ? "https://api.anthropic.com"
              : entry.name === "openai"
                ? "https://api.openai.com/v1"
                : entry.name === "local"
                  ? "http://localhost:11434/v1"
                  : "https://your-provider.com/v1"
          }
          className="w-full h-8 px-3 text-[13px] bg-[#f5f5f7] rounded-lg border border-transparent focus:border-[#0a84ff] focus:outline-none placeholder:text-[#b0b0b5]"
        />
      </div>

      {/* API Key */}
      <div className="space-y-1">
        <label className="text-[11px] text-[#6e6e73] font-medium uppercase tracking-wide">
          API Key
        </label>
        <input
          type="password"
          value={keyInput}
          onChange={(e) => setKeyInput(e.target.value)}
          placeholder={hasKey ? `•••• key saved (${saved!.maskedKey})` : "Enter API key"}
          autoComplete="new-password"
          className="w-full h-8 px-3 text-[13px] bg-[#f5f5f7] rounded-lg border border-transparent focus:border-[#0a84ff] focus:outline-none placeholder:text-[#b0b0b5]"
        />
        <div className="flex items-center gap-1 text-[11px] text-[#86868b]">
          <Key className="w-3 h-3" />
          <span>Stored in macOS Keychain — never written to disk or logs</span>
        </div>
      </div>

      {/* Error */}
      {error && (
        <p className="text-[12px] text-[#ff3b30]">{error}</p>
      )}

      {/* Save button */}
      <button
        onClick={handleSave}
        disabled={saving}
        className="h-8 px-4 text-[13px] font-medium bg-[#0a84ff] text-white rounded-lg hover:bg-[#0070d8] disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {saving ? "Saving…" : "Save"}
      </button>
    </div>
  );
}

// ── Main component ───────────────────────────────────────────────────────────

export function Settings({ onClose }: SettingsProps) {
  const [providers, setProviders] = useState<Provider[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Fetch the provider list. StrictMode-safe via a per-invocation `mounted`
  // flag captured in the closure (a ref would be reset to true by the second
  // StrictMode invocation and allow the first resolve to fire).
  function fetchProviders() {
    let mounted = true;
    ipc.provider
      .list()
      .then((list) => {
        if (mounted) {
          setProviders(list);
          setLoadError(null);
        }
      })
      .catch((err: unknown) => {
        if (mounted) {
          setLoadError(err instanceof Error ? err.message : String(err));
        }
      });
    return () => {
      mounted = false;
    };
  }

  useEffect(fetchProviders, []);

  // Escape key closes the overlay (cleanup removes the listener on unmount).
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  function getSaved(name: ProviderName): Provider | undefined {
    return providers.find((p) => p.name === name);
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[560px] max-h-[90vh] bg-white rounded-2xl shadow-2xl ring-1 ring-black/[0.08] flex flex-col overflow-hidden">

        {/* ── Header ── */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-black/[0.06] shrink-0">
          <span className="text-[13px] font-semibold tracking-tight text-[#1d1d1f]">
            Settings
          </span>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
            aria-label="Close settings"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Scrollable body ── */}
        <div className="flex-1 overflow-y-auto px-6 py-5 space-y-5 min-h-0">

          {/* Providers section */}
          <div className="space-y-3">
            <h2 className="text-[11px] font-semibold text-[#6e6e73] uppercase tracking-widest">
              Providers
            </h2>

            {loadError && (
              <p className="text-[12px] text-[#ff3b30] bg-[#fff5f5] border border-[#ffd0d0] rounded-lg px-3 py-2">
                Failed to load providers: {loadError}
              </p>
            )}

            {PROVIDERS.map((entry) => (
              <ProviderCard
                key={entry.name}
                entry={entry}
                saved={getSaved(entry.name)}
                onSaved={fetchProviders}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
