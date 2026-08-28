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
          aria-label={label}
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
        Restrict which tools, file paths and shell programs this agent can
        reach. Leave a list empty to allow everything in that category. Every
        control here is checked by the runtime before a tool call runs.
      </p>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Approval</h4>
        <Toggle
          checked={value.requireApprovalOnWrite}
          onChange={v => set("requireApprovalOnWrite", v)}
          label="Require human approval before every write"
        />
        <p className="form-field__helper" style={{ marginTop: 4 }}>
          The run pauses until you approve or reject each write in the Approvals
          queue. Custom shell tools count as writes — a command can change
          anything the app can.
        </p>
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Allowed Tools</h4>
        <TagList
          label="Tool allowlist"
          helperText="Exact agent tool names (e.g. file_write, file_read). Empty = all tools allowed."
          items={value.allowedTools}
          placeholder="file_write"
          onChange={v => set("allowedTools", v)}
        />
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">File Paths</h4>
        <TagList
          label="Allowed write paths"
          helperText="Directories the agent may write to. Relative entries resolve against the workspace root. Empty = no restriction."
          items={value.allowFileWritePaths}
          placeholder="src/"
          onChange={v => set("allowFileWritePaths", v)}
        />
        <TagList
          label="Allowed read paths"
          helperText="Directories the agent may read from, via file_read and file_list. Empty = no restriction."
          items={value.allowFileReadPaths}
          placeholder="docs/"
          onChange={v => set("allowFileReadPaths", v)}
        />
        <p className="form-field__helper" style={{ marginTop: 4 }}>
          Setting either list also blocks custom shell tools: a command runs
          outside these paths and offers nothing to check, so it is refused
          rather than let through unchecked.
        </p>
      </div>

      <div className="perm-editor__section">
        <h4 className="perm-editor__section-title">Shell</h4>
        <TagList
          label="Allowed shell programs"
          helperText="Program names a custom shell tool may run (e.g. git, npm). Matched against the program, not the arguments. Empty = no restriction."
          items={value.allowShellCommands}
          placeholder="git"
          onChange={v => set("allowShellCommands", v)}
        />
      </div>
    </div>
  );
}
