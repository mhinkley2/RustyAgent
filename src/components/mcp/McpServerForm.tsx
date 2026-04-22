import { useEffect, useState } from "react";
import {
  SlidePanel,
  FormField,
  TextInput,
  Textarea,
  Toggle,
  NumberInput,
  KeyValueInput,
  type KeyValuePair,
} from "../forms";
import type { McpServer, CreateMcpServerInput } from "../../types/mcp";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeId() {
  return Math.random().toString(36).slice(2, 9);
}

function argsToString(args: string[]): string {
  return args.join("\n");
}

function stringToArgs(raw: string): string[] {
  return raw
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
}

function envToKV(env: Record<string, string>): KeyValuePair[] {
  return Object.entries(env).map(([key, value]) => ({
    id: makeId(),
    key,
    value,
  }));
}

function kvToEnv(pairs: KeyValuePair[]): Record<string, string> {
  const result: Record<string, string> = {};
  for (const { key, value } of pairs) {
    if (key.trim()) result[key.trim()] = value;
  }
  return result;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface McpServerFormProps {
  open: boolean;
  editing: McpServer | null;
  onClose: () => void;
  onSave: (input: CreateMcpServerInput) => Promise<void>;
}

export function McpServerForm({
  open,
  editing,
  onClose,
  onSave,
}: McpServerFormProps) {
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [argsRaw, setArgsRaw] = useState("");
  const [envPairs, setEnvPairs] = useState<KeyValuePair[]>([]);
  const [autoRestart, setAutoRestart] = useState(true);
  const [maxRestarts, setMaxRestarts] = useState(3);
  const [saving, setSaving] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);

  // Populate form when editing changes.
  useEffect(() => {
    if (editing) {
      setName(editing.name);
      setCommand(editing.command);
      setArgsRaw(argsToString(editing.args));
      setEnvPairs(envToKV(editing.env_vars));
      setAutoRestart(editing.auto_restart);
      setMaxRestarts(editing.max_restart_attempts);
    } else {
      setName("");
      setCommand("");
      setArgsRaw("");
      setEnvPairs([]);
      setAutoRestart(true);
      setMaxRestarts(3);
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
        command: command.trim(),
        args: stringToArgs(argsRaw),
        env_vars: kvToEnv(envPairs),
        auto_restart: autoRestart,
        max_restart_attempts: maxRestarts,
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
      title={editing ? "Edit MCP Server" : "Add MCP Server"}
      width={540}
      footer={
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn btn--primary" onClick={handleSave} disabled={saving}>
            {saving ? "Saving…" : editing ? "Save changes" : "Add server"}
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

        {/* ── Connection ────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Connection</h3>

          <FormField label="Name" required>
            {(id) => (
              <TextInput
                id={id}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. filesystem"
                autoFocus
              />
            )}
          </FormField>

          <FormField
            label="Command"
            required
            helperText="Executable that will be launched as the MCP server process."
          >
            {(id) => (
              <TextInput
                id={id}
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                placeholder="e.g. npx or /usr/bin/python3"
                style={{ fontFamily: "var(--font-mono)" }}
              />
            )}
          </FormField>

          <FormField label="Arguments" helperText="One argument per line.">
            {(id) => (
              <Textarea
                id={id}
                rows={3}
                value={argsRaw}
                onChange={(e) => setArgsRaw(e.target.value)}
                placeholder={"@modelcontextprotocol/server-filesystem\n/tmp"}
                mono
              />
            )}
          </FormField>
        </section>

        {/* ── Environment ───────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Environment</h3>

          <FormField label="Environment variables">
            {() => (
              <KeyValueInput
                pairs={envPairs}
                onChange={setEnvPairs}
                keyPlaceholder="VAR_NAME"
                valuePlaceholder="value"
                addLabel="+ Add variable"
              />
            )}
          </FormField>
        </section>

        {/* ── Behavior ──────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Behavior</h3>

          <FormField
            label="Auto-restart on crash"
            labelAs="div"
            helperText="Restart the server process if it exits unexpectedly."
          >
            {() => (
              <Toggle
                checked={autoRestart}
                onChange={setAutoRestart}
                label="Enable auto-restart"
              />
            )}
          </FormField>

          {autoRestart && (
            <FormField label="Max restart attempts">
              {(id) => (
                <NumberInput
                  id={id}
                  value={maxRestarts}
                  onChange={(v) => setMaxRestarts(typeof v === "number" ? Math.max(1, v) : 1)}
                  min={1}
                  max={20}
                />
              )}
            </FormField>
          )}
        </section>
      </div>
    </SlidePanel>
  );
}
