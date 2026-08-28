import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PermissionEditor } from "./PermissionEditor";
import { defaultPermissions } from "../../types/permissions";
import type { AgentPermissions } from "../../types/permissions";

function renderEditor(overrides: Partial<AgentPermissions> = {}) {
  const value: AgentPermissions = { ...defaultPermissions("agent-1"), ...overrides };
  const onChange = vi.fn();
  render(<PermissionEditor value={value} onChange={onChange} />);
  return { value, onChange };
}

describe("PermissionEditor", () => {
  it("offers only controls the runtime actually enforces", () => {
    renderEditor();

    expect(screen.getByLabelText("Tool allowlist")).toBeInTheDocument();
    expect(screen.getByLabelText("Allowed write paths")).toBeInTheDocument();
    expect(screen.getByLabelText("Allowed read paths")).toBeInTheDocument();
    expect(screen.getByLabelText("Allowed shell programs")).toBeInTheDocument();
    expect(screen.getByRole("switch")).toBeInTheDocument();
  });

  // The whole point of the change: a control the runtime never consults must
  // not be offered, because an operator configures it and then trusts it.
  it("no longer offers a network host allow-list", () => {
    renderEditor();

    expect(screen.queryByLabelText(/network/i)).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText("api.github.com")).not.toBeInTheDocument();
  });

  it("adds a read path and reports it upward", async () => {
    const user = userEvent.setup();
    const { onChange } = renderEditor();

    const field = screen.getByLabelText("Allowed read paths");
    await user.type(field, "docs/{Enter}");

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ allowFileReadPaths: ["docs/"] })
    );
  });

  it("removes an entry that is already configured", async () => {
    const user = userEvent.setup();
    const { onChange } = renderEditor({ allowShellCommands: ["git", "npm"] });

    await user.click(screen.getByRole("button", { name: "Remove git" }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ allowShellCommands: ["npm"] })
    );
  });

  it("says an empty list is unrestricted", () => {
    renderEditor();

    expect(
      screen.getAllByText(/is unrestricted$/).length
    ).toBeGreaterThanOrEqual(4);
  });

  it("toggles the write-approval requirement", async () => {
    const user = userEvent.setup();
    const { onChange } = renderEditor();

    await user.click(screen.getByRole("switch"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ requireApprovalOnWrite: true })
    );
  });
});
