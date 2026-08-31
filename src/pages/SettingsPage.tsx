import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PageHeader } from "../components/board/PageHeader";
import { useSettings } from "../hooks/useSettings";
import type { AppSettings, NotificationSettings } from "../types/settings";
import { DEFAULT_NOTIFICATIONS } from "../types/settings";
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

function ToggleField({
  id,
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  hint: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className={`settings-toggle${disabled ? " settings-toggle--disabled" : ""}`}>
      <input
        id={id}
        className="settings-toggle__input"
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <label className="settings-toggle__label" htmlFor={id}>
        <span className="settings-toggle__title">{label}</span>
        <span className="settings-toggle__hint">{hint}</span>
      </label>
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
    approval_timeout_secs: "",
  });
  const [notifications, setNotifications] =
    useState<NotificationSettings>(DEFAULT_NOTIFICATIONS);
  const [saved, setSaved] = useState(false);
  /**
   * The version this build was stamped with.
   *
   * Read at runtime rather than baked into the bundle, so it is the same
   * number on the installer's filename — the one a bug report should quote.
   */
  const [appVersion, setAppVersion] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setAppVersion)
      // A missing version is not worth a toast; the row simply says so.
      .catch(() => setAppVersion(null));
  }, []);

  // Populate draft once settings load.
  useEffect(() => {
    if (!loading) {
      setDraft({
        anthropic_api_key: settings.anthropic_api_key ?? "",
        openrouter_api_key: settings.openrouter_api_key ?? "",
        deepseek_api_key: settings.deepseek_api_key ?? "",
        ollama_base_url: settings.ollama_base_url ?? "",
        approval_timeout_secs:
          settings.approval_timeout_secs == null
            ? ""
            : String(settings.approval_timeout_secs),
      });
      setNotifications(settings.notifications ?? DEFAULT_NOTIFICATIONS);
    }
  }, [loading]); // eslint-disable-line react-hooks/exhaustive-deps

  const set = (key: keyof typeof draft, value: string) => {
    setSaved(false);
    setDraft((prev) => ({ ...prev, [key]: value }));
  };

  const setNotification = (key: keyof NotificationSettings, value: boolean) => {
    setSaved(false);
    setNotifications((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async () => {
    const timeout = Number.parseInt(draft.approval_timeout_secs, 10);
    const toSave: AppSettings = {
      // Spread what was loaded so knobs with no control here — event
      // retention, pipeline parallelism — survive a save from this page
      // instead of being erased back to their defaults.
      ...settings,
      anthropic_api_key: draft.anthropic_api_key || null,
      openrouter_api_key: draft.openrouter_api_key || null,
      deepseek_api_key: draft.deepseek_api_key || null,
      ollama_base_url: draft.ollama_base_url || null,
      notifications,
      approval_timeout_secs:
        Number.isFinite(timeout) && timeout > 0 ? timeout : null,
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

        {/* ── Notifications ──────────────────────────────────────── */}
        <section className="settings-section">
          <h2 className="settings-section__title">Notifications</h2>
          <p className="settings-section__desc">
            How RustyAgent reaches you while a run is working and you are not
            watching. Notifications are raised once per event — never per token
            or per tool call.
          </p>

          <ToggleField
            id="notify-enabled"
            label="Desktop notifications"
            hint="Master switch. With this off, nothing below is delivered."
            checked={notifications.enabled}
            onChange={(v) => setNotification("enabled", v)}
          />

          <ToggleField
            id="notify-approval"
            label="A run needs your approval"
            hint="A gated tool call parks the run until you decide, so this is the one that has to reach you."
            checked={notifications.onApproval}
            disabled={!notifications.enabled}
            onChange={(v) => setNotification("onApproval", v)}
          />

          <ToggleField
            id="notify-failed"
            label="A run fails"
            hint="Raised once, when the run reaches a failed state."
            checked={notifications.onRunFailed}
            disabled={!notifications.enabled}
            onChange={(v) => setNotification("onRunFailed", v)}
          />

          <ToggleField
            id="notify-completed"
            label="A run finishes"
            hint="Raised once, when the run completes. Cancelling a run yourself never notifies."
            checked={notifications.onRunCompleted}
            disabled={!notifications.enabled}
            onChange={(v) => setNotification("onRunCompleted", v)}
          />

          <ToggleField
            id="notify-agent"
            label="An agent asks to notify you"
            hint="The send_notification tool. With this off the tool reports the failure to the agent rather than pretending you were told."
            checked={notifications.onAgentRequest}
            disabled={!notifications.enabled}
            onChange={(v) => setNotification("onAgentRequest", v)}
          />

          <div className="settings-field">
            <label className="settings-field__label" htmlFor="approval-timeout">
              Approval timeout (seconds)
            </label>
            <input
              id="approval-timeout"
              className="settings-field__input"
              type="number"
              min={0}
              value={draft.approval_timeout_secs}
              onChange={(e) => set("approval_timeout_secs", e.target.value)}
              placeholder="Wait indefinitely"
              autoComplete="off"
            />
            <p className="settings-field__hint">
              Leave blank to wait indefinitely, so a run parks until you come
              back. A value here ends the wait instead — recorded as expired,
              never as a decision you made.
            </p>
          </div>
        </section>

        {/* ── About ──────────────────────────────────────────────── */}
        <section className="settings-section">
          <h2 className="settings-section__title">About</h2>
          <p className="settings-section__desc">
            Quote this version in a bug report. It is the same number on the
            installer that produced this build.
          </p>

          <div className="settings-field">
            <span className="settings-field__label">Version</span>
            <p className="settings-field__hint">
              RustyAgent{" "}
              <code className="settings-section__path">
                {appVersion ?? "unknown"}
              </code>
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
