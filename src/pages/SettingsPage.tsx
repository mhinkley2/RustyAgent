import { useState, useEffect } from "react";
import { PageHeader } from "../components/board/PageHeader";
import { useSettings } from "../hooks/useSettings";
import type { AppSettings } from "../types/settings";
import { Eye, EyeOff, Check } from "lucide-react";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function ApiKeyField({
  id,
  label,
  hint,
  value,
  onChange,
}: {
  id: string;
  label: string;
  hint: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const [visible, setVisible] = useState(false);

  return (
    <div className="settings-field">
      <label className="settings-field__label" htmlFor={id}>
        {label}
      </label>
      <div className="settings-field__key-wrap">
        <input
          id={id}
          className="settings-field__input settings-field__input--mono"
          type={visible ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={visible ? "Paste API key…" : "••••••••••••••••"}
          autoComplete="off"
          spellCheck={false}
        />
        <button
          type="button"
          className="settings-field__eye"
          onClick={() => setVisible((v) => !v)}
          aria-label={visible ? "Hide key" : "Show key"}
          title={visible ? "Hide" : "Show"}
        >
          {visible ? <EyeOff size={14} /> : <Eye size={14} />}
        </button>
      </div>
      <p className="settings-field__hint">{hint}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// SettingsPage
// ---------------------------------------------------------------------------

export default function SettingsPage() {
  const { settings, loading, saving, error, saveSettings } = useSettings();

  // Local draft — edited freely, saved only on button click.
  const [draft, setDraft] = useState({
    anthropic_api_key: "",
    openrouter_api_key: "",
    deepseek_api_key: "",
    ollama_base_url: "",
  });
  const [saved, setSaved] = useState(false);

  // Populate draft once settings load.
  useEffect(() => {
    if (!loading) {
      setDraft({
        anthropic_api_key: settings.anthropic_api_key ?? "",
        openrouter_api_key: settings.openrouter_api_key ?? "",
        deepseek_api_key: settings.deepseek_api_key ?? "",
        ollama_base_url: settings.ollama_base_url ?? "",
      });
    }
  }, [loading]); // eslint-disable-line react-hooks/exhaustive-deps

  const set = (key: keyof typeof draft, value: string) => {
    setSaved(false);
    setDraft((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async () => {
    const toSave: AppSettings = {
      anthropic_api_key: draft.anthropic_api_key || null,
      openrouter_api_key: draft.openrouter_api_key || null,
      deepseek_api_key: draft.deepseek_api_key || null,
      ollama_base_url: draft.ollama_base_url || null,
    };
    await saveSettings(toSave);
    setSaved(true);
    setTimeout(() => setSaved(false), 2500);
  };

  if (loading) {
    return (
      <div className="settings-page">
        <PageHeader title="Settings" sticky />
        <div className="settings-page__loading">Loading settings…</div>
      </div>
    );
  }

  const saveButton = (
    <button
      className="btn btn--primary btn--sm"
      onClick={handleSave}
      disabled={saving}
    >
      {saved ? (
        <>
          <Check size={14} />
          &nbsp;Saved
        </>
      ) : saving ? (
        "Saving…"
      ) : (
        "Save settings"
      )}
    </button>
  );

  return (
    <div className="settings-page">
      <PageHeader title="Settings" cta={saveButton} sticky />

      <div className="settings-page__content">
        {/* ── LLM Providers ──────────────────────────────────────── */}
        <section className="settings-section">
          <h2 className="settings-section__title">LLM Providers</h2>
          <p className="settings-section__desc">
            API keys are stored in{" "}
            <code className="settings-section__path">settings.json</code> in
            the app data directory — never sent anywhere except the respective
            API servers.
          </p>

          <ApiKeyField
            id="anthropic-key"
            label="Anthropic API key"
            hint="Used by agent profiles with provider = Anthropic. Get yours at console.anthropic.com."
            value={draft.anthropic_api_key}
            onChange={(val) => set("anthropic_api_key", val)}
          />

          <ApiKeyField
            id="openrouter-key"
            label="OpenRouter API key"
            hint="Used by agent profiles with provider = OpenRouter. Get yours at openrouter.ai/keys."
            value={draft.openrouter_api_key}
            onChange={(val) => set("openrouter_api_key", val)}
          />

          <ApiKeyField
            id="deepseek-key"
            label="DeepSeek API key"
            hint="Used by agent profiles with provider = DeepSeek. Get yours at platform.deepseek.com."
            value={draft.deepseek_api_key}
            onChange={(val) => set("deepseek_api_key", val)}
          />
        </section>

        {/* ── Ollama ─────────────────────────────────────────────── */}
        <section className="settings-section">
          <h2 className="settings-section__title">Ollama</h2>

          <div className="settings-field">
            <label className="settings-field__label" htmlFor="ollama-url">
              Ollama base URL
            </label>
            <input
              id="ollama-url"
              className="settings-field__input settings-field__input--mono"
              type="url"
              value={draft.ollama_base_url}
              onChange={(e) => set("ollama_base_url", e.target.value)}
              placeholder="http://localhost:11434"
              autoComplete="off"
            />
            <p className="settings-field__hint">
              Leave blank to use the default (http://localhost:11434). Change
              this if you run Ollama on a different host or port.
            </p>
          </div>
        </section>

        {/* ── Save bar ───────────────────────────────────────────── */}
        {error && (
          <div className="settings-save-bar">
            <span className="settings-save-bar__error">{error}</span>
          </div>
        )}
      </div>
    </div>
  );
}
