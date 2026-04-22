import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useEffect } from "react";
import { notifyError } from "../components/ui/Toast";
import type { AppSettings } from "../types/settings";
import { DEFAULT_SETTINGS } from "../types/settings";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface UseSettingsReturn {
  settings: AppSettings;
  loading: boolean;
  saving: boolean;
  error: string | null;
  saveSettings: (updated: AppSettings) => Promise<void>;
}

export function useSettings(): UseSettingsReturn {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<AppSettings>("get_settings")
      .then(setSettings)
      .catch((e) => {
        const message = errorMessage(e);
        setError(message);
        notifyError("Failed to load settings", message, { duration: 7000 });
      })
      .finally(() => setLoading(false));
  }, []);

  const saveSettings = useCallback(async (updated: AppSettings) => {
    setSaving(true);
    setError(null);
    try {
      await invoke("save_settings", { settings: updated });
      setSettings(updated);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      notifyError("Failed to save settings", message, { duration: 7000 });
      throw e;
    } finally {
      setSaving(false);
    }
  }, []);

  return { settings, loading, saving, error, saveSettings };
}
