import { useState, KeyboardEvent } from "react";
import type { AgentPermissions } from "../../types/permissions";
import { Toggle } from "../forms";
import { X, Plus } from "lucide-react";

// ---------------------------------------------------------------------------
// TagList — simple add/remove chip list for a string[]
// ---------------------------------------------------------------------------

interface TagListProps {
  label: string;
  helperText?: string;
  items: string[];
  placeholder?: string;
  onChange: (next: string[]) => void;
}

function TagList({ label, helperText, items, placeholder, onChange }: TagListProps) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const v = draft.trim();
    if (!v || items.includes(v)) return;
    onChange([...items, v]);
    setDraft("");
  };

  const remove = (i: number) => onChange(items.filter((_, idx) => idx !== i));

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") { e.preventDefault(); add(); }
    if (e.key === "Backspace" && draft === "" && items.length > 0) {
      remove(items.length - 1);
    }
  };

  return (
    <div className="perm-tag-list">
      <label className="form-field__label">{label}</label>
      {helperText && <p className="form-field__helper">{helperText}</p>}
      <div className="perm-tag-list__input-row">
        {items.map((item, i) => (
          <span key={i} className="perm-tag">
            <span className="perm-tag__text">{item}</span>
            <button
              type="button"
              className="perm-tag__remove"
              onClick={() => remove(i)}
              aria-label={`Remove ${item}`}
            >
              <X size={10} />
            </button>
          </span>
        ))}
        <input
          type="text"
          className="perm-tag-list__draft"
          value={draft}
          onChange={e => setDraft(e.target.value)}
          onKeyDown={onKey}
          placeholder={items.length === 0 ? placeholder : ""}
        />
        {draft.trim() && (
          <button type="button" className="perm-tag-list__add-btn" onClick={add} aria-label="Add">
            <Plus size={12} />
          </button>
        )}
      </div>
      {items.length === 0 && (
        <p className="perm-tag-list__empty">No entries — {label.toLowerCase()} is unrestricted</p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// PermissionEditor
// ---------------------------------------------------------------------------

interface PermissionEditorProps {
  value: AgentPermissions;
  onChange: (next: AgentPermissions) => void;
}

export function PermissionEditor({ value, onChange }: PermissionEditorProps) {
  const set = <K extends keyof AgentPermissions>(key: K, v: AgentPermissions[K]) =>
    onChange({ ...value, [key]: v });

  return (
    <div className="perm-editor">
      <p className="perm-editor__intro">
        Restrict which tools and file paths this agent can access. Leave a list
        empty to allow everything in that category.
      </p>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Approval</h4>
        <Toggle
          checked={value.requireApprovalOnWrite}
          onChange={v => set("requireApprovalOnWrite", v)}
          label="Require human approval before every file write"
        />
        <p className="form-field__helper" style={{ marginTop: 4 }}>
          The run will pause and wait until you approve or reject each write in
          the Approvals queue.
        </p>
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Allowed Tools</h4>
        <TagList
          label="Tool allowlist"
          helperText="Exact built-in tool names (e.g. write_file_text). Empty = all tools allowed."
          items={value.allowedTools}
          placeholder="write_file_text"
          onChange={v => set("allowedTools", v)}
        />
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">File Paths</h4>
        <TagList
          label="Allowed write paths"
          helperText="Absolute directory prefixes the agent may write to (e.g. /workspace/). Empty = no restriction."
          items={value.allowFileWritePaths}
          placeholder="/workspace/"
          onChange={v => set("allowFileWritePaths", v)}
        />
        <TagList
          label="Allowed read paths"
          helperText="Absolute directory prefixes the agent may read from. Empty = no restriction."
          items={value.allowFileReadPaths}
          placeholder="/workspace/"
          onChange={v => set("allowFileReadPaths", v)}
        />
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Shell &amp; Network</h4>
        <TagList
          label="Allowed shell commands"
          helperText="Command name prefixes (e.g. git, npm). Empty = no restriction."
          items={value.allowShellCommands}
          placeholder="git"
          onChange={v => set("allowShellCommands", v)}
        />
        <TagList
          label="Allowed network hosts"
          helperText="Hostname allow-list (e.g. api.github.com). Empty = no restriction."
          items={value.allowNetworkHosts}
          placeholder="api.github.com"
          onChange={v => set("allowNetworkHosts", v)}
        />
      </div>
    </div>
  );
}
