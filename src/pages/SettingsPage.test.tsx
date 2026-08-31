import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { tauriMock } from "../test/tauriMock";
import SettingsPage from "./SettingsPage";
import type { AppSettings } from "../types/settings";

function renderPage(settings: Partial<AppSettings> = {}) {
  const saved: AppSettings[] = [];
  tauriMock.handleAll({
    get_settings: () => ({
      anthropic_api_key: null,
      openrouter_api_key: null,
      deepseek_api_key: null,
      ollama_base_url: null,
      ...settings,
    }),
    save_settings: (args) => {
      saved.push(args.settings as AppSettings);
      return null;
    },
    get_app_version: () => "0.2.0",
  });
  render(<SettingsPage />);
  return saved;
}

async function save() {
  await userEvent.click(screen.getByRole("button", { name: /save settings/i }));
}

describe("SettingsPage version", () => {
  // Every build was 0.1.0 for four local releases, so a bug report naming a
  // version said nothing. The number has to be reachable without hunting for
  // an installer filename.
  it("shows the version this build was stamped with", async () => {
    renderPage();

    expect(await screen.findByText("0.2.0")).toBeInTheDocument();
  });

  // The version is a nicety; failing to read it must not take the settings
  // page down with it, since the page is also where the API keys live.
  it("says unknown rather than failing when the version cannot be read", async () => {
    tauriMock.handleAll({
      get_settings: () => ({
        anthropic_api_key: null,
        openrouter_api_key: null,
        deepseek_api_key: null,
        ollama_base_url: null,
      }),
      save_settings: () => null,
      get_app_version: () => {
        throw new Error("no app handle");
      },
    });
    render(<SettingsPage />);

    expect(await screen.findByText("unknown")).toBeInTheDocument();
    // And the rest of the page still works.
    expect(screen.getByLabelText(/anthropic api key/i)).toBeInTheDocument();
  });
});

describe("SettingsPage notifications", () => {
  it("defaults every category to on when settings.json predates the feature", async () => {
    renderPage();

    await waitFor(() =>
      expect(screen.getByLabelText(/desktop notifications/i)).toBeChecked(),
    );
    expect(screen.getByLabelText(/needs your approval/i)).toBeChecked();
    expect(screen.getByLabelText(/a run fails/i)).toBeChecked();
    expect(screen.getByLabelText(/a run finishes/i)).toBeChecked();
  });

  it("saves a category the user switched off", async () => {
    const saved = renderPage();
    await waitFor(() => screen.getByLabelText(/a run finishes/i));

    await userEvent.click(screen.getByLabelText(/a run finishes/i));
    await save();

    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].notifications).toMatchObject({
      enabled: true,
      onApproval: true,
      onRunCompleted: false,
    });
  });

  it("disables the categories under the master switch without clearing them", async () => {
    const saved = renderPage();
    await waitFor(() => screen.getByLabelText(/desktop notifications/i));

    await userEvent.click(screen.getByLabelText(/desktop notifications/i));

    // Still shown as configured — turning the master switch back on must not
    // look like it reset every preference underneath.
    expect(screen.getByLabelText(/needs your approval/i)).toBeDisabled();
    expect(screen.getByLabelText(/needs your approval/i)).toBeChecked();

    await save();
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].notifications).toMatchObject({ enabled: false, onApproval: true });
  });

  it("leaves the approval timeout unset so a parked run waits indefinitely", async () => {
    const saved = renderPage();
    await waitFor(() => screen.getByLabelText(/approval timeout/i));

    expect(screen.getByLabelText(/approval timeout/i)).toHaveValue(null);

    await save();
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].approval_timeout_secs).toBeNull();
  });

  it("saves a configured approval timeout", async () => {
    const saved = renderPage();
    await waitFor(() => screen.getByLabelText(/approval timeout/i));

    await userEvent.type(screen.getByLabelText(/approval timeout/i), "900");
    await save();

    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].approval_timeout_secs).toBe(900);
  });

  it("treats a zero timeout as unset rather than as instant expiry", async () => {
    const saved = renderPage();
    await waitFor(() => screen.getByLabelText(/approval timeout/i));

    await userEvent.type(screen.getByLabelText(/approval timeout/i), "0");
    await save();

    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].approval_timeout_secs).toBeNull();
  });

  // This page edits four keys and now writes back the whole file. Without
  // carrying the rest through, saving an API key from here would silently
  // reset knobs that only exist in settings.json.
  it("preserves settings it has no control for", async () => {
    const saved = renderPage({ event_retention_runs: 25, max_parallel_steps: 3 });
    await waitFor(() => screen.getByLabelText(/anthropic api key/i));

    await userEvent.type(screen.getByLabelText(/anthropic api key/i), "sk-ant-x");
    await save();

    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].event_retention_runs).toBe(25);
    expect(saved[0].max_parallel_steps).toBe(3);
    expect(saved[0].anthropic_api_key).toBe("sk-ant-x");
  });
});
