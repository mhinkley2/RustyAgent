import { useEffect, useState } from "react";
import {
  SlidePanel,
  FormField,
  TextInput,
  Textarea,
  NumberInput,
} from "../forms";
import type { CustomTool, CreateCustomToolInput } from "../../types/custom_tools";

interface CustomToolFormProps {
  open: boolean;
  editing: CustomTool | null;
  onClose: () => void;
  onSave: (input: CreateCustomToolInput) => Promise<void>;
}

export function CustomToolForm({ open, editing, onClose, onSave }: CustomToolFormProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [command, setCommand] = useState("");
  const [workingDir, setWorkingDir] = useState(".");
  const [timeoutSecs, setTimeoutSecs] = useState(30);
  const [saving, setSaving] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);

  useEffect(() => {
    if (editing) {
      setName(editing.name);
      setDescription(editing.description);
      setCommand(editing.command);
      setWorkingDir(editing.working_dir);
      setTimeoutSecs(editing.timeout_secs);
    } else {
      setName("");
      setDescription("");
      setCommand("");
      setWorkingDir(".");
      setTimeoutSecs(30);
    }
    setFieldError(null);
    setSaving(false);
  }, [editing, open]);

  const handleSave = async () => {
    if (!name.trim()) { setFieldError("Name is required."); return; }
    if (!command.trim()) { setFieldError("Command is required."); return; }
    setFieldError(null);
    setSaving(true);
    try {
      await onSave({
        name: name.trim(),
        description: description.trim() || undefined,
        command: command.trim(),
        working_dir: workingDir.trim() || ".",
        timeout_secs: timeoutSecs,
      });
      onClose();
    } catch (e) {
      setFieldError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SlidePanel
      open={open}
      onClose={onClose}
      title={editing ? "Edit Shell Tool" : "New Shell Tool"}
      width={540}
      footer={
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn btn--primary" onClick={handleSave} disabled={saving}>
            {saving ? "Saving…" : editing ? "Save changes" : "Create tool"}
          </button>
          <button className="btn btn--ghost" onClick={onClose} disabled={saving}>
            Cancel
          </button>
        </div>
      }
    >
      <div className="agent-form">
        {fieldError && (
          <div className="mcp-form__error" style={{ marginBottom: 8 }}>{fieldError}</div>
        )}

        {/* ── Identity ──────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Identity</h3>

          <FormField
            label="Name"
            required
            helperText="Tool name the agent will call — use underscores, no spaces (e.g. run_tests)."
          >
            {(id) => (
              <TextInput
                id={id}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="run_tests"
                spellCheck={false}
                autoFocus
              />
            )}
          </FormField>

          <FormField
            label="Description"
            helperText="Shown to the LLM — describe when and why to use this tool."
          >
            {(id) => (
              <Textarea
                id={id}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Run the test suite and return results."
                rows={3}
              />
            )}
          </FormField>
        </section>

        {/* ── Execution ─────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Execution</h3>

          <FormField
            label="Command"
            required
            helperText="Arguments split by whitespace — no shell interpolation."
          >
            {(id) => (
              <TextInput
                id={id}
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                placeholder="cargo test --workspace"
                spellCheck={false}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            )}
          </FormField>

          <FormField
            label="Working directory"
            helperText="Relative to workspace root. Use . for the root."
          >
            {(id) => (
              <TextInput
                id={id}
                value={workingDir}
                onChange={(e) => setWorkingDir(e.target.value)}
                placeholder="."
                spellCheck={false}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            )}
          </FormField>

          <FormField
            label="Timeout (seconds)"
            helperText="Maximum run time before the command is killed. Default: 30 s."
          >
            {(id) => (
              <NumberInput
                id={id}
                value={timeoutSecs}
                onChange={(v) => setTimeoutSecs(typeof v === "number" ? v : 30)}
                min={1}
                max={3600}
              />
            )}
          </FormField>
        </section>
      </div>
    </SlidePanel>
  );
}
