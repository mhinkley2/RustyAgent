import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useWorkspaceContext } from "../../context/WorkspaceContext";
import {
  SlidePanel,
  FormField,
  TextInput,
  Textarea,
  FormSelect,
  Toggle,
  NumberInput,
} from "../forms";
import type { SelectOption } from "../forms";
import type { AgentProfile, CreateProfileInput, Provider, RunMode, ContextStrategy } from "../../types/agent";
import {
  PROVIDER_MODELS,
  PROVIDER_LABELS,
  CONTEXT_STRATEGY_OPTIONS,
} from "../../types/agent";
import { useMcpServers, useToolBindings } from "../../hooks/useMcpServers";
import { useCustomTools, useCustomToolBindings } from "../../hooks/useCustomTools";
import { useAgentPermissions } from "../../hooks/useAgentPermissions";
import type { AgentPermissions } from "../../types/permissions";
import { defaultPermissions } from "../../types/permissions";
import { PermissionEditor } from "./PermissionEditor";
import { Unlink } from "lucide-react";

// ---------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------

interface FormState {
  name: string;
  description: string;
  system_prompt: string;
  provider: Provider | "";
  model: string;
  context_strategy: ContextStrategy;
  persistent_memory: boolean;
  max_input_tokens: number | "";
  max_output_tokens: number | "";
  run_mode: RunMode;
  cron_expression: string;
  continuous_poll_interval_secs: number | "";
  max_iterations: number | "";
  max_retries: number | "";
  scope: "global" | "workspace";
}

type FormErrors = Partial<Record<keyof FormState, string>>;

const DEFAULT_FORM: FormState = {
  name: "",
  description: "",
  system_prompt: "",
  provider: "",
  model: "",
  context_strategy: "recent",
  persistent_memory: false,
  max_input_tokens: "",
  max_output_tokens: "",
  run_mode: "manual",
  cron_expression: "",
  continuous_poll_interval_secs: 30,
  max_iterations: 20,
  max_retries: 2,
  scope: "global",
};

function profileToForm(p: AgentProfile): FormState {
  return {
    name:                         p.name,
    description:                  p.description ?? "",
    system_prompt:                p.system_prompt,
    provider:                     p.provider as Provider,
    model:                        p.model,
    context_strategy:             p.context_strategy as ContextStrategy,
    persistent_memory:            p.persistent_memory,
    max_input_tokens:             p.max_input_tokens ?? "",
    max_output_tokens:            p.max_output_tokens ?? "",
    run_mode:                     p.run_mode as RunMode,
    cron_expression:              p.cron_expression ?? "",
    continuous_poll_interval_secs: p.continuous_poll_interval_secs,
    max_iterations:               p.max_iterations,
    max_retries:                  p.max_retries,
    scope:                        p.scope ?? "global",
  };
}

function validate(form: FormState): FormErrors {
  const errors: FormErrors = {};
  if (!form.name.trim()) errors.name = "Name is required";
  if (!form.provider) errors.provider = "Provider is required";
  if (!form.model.trim()) errors.model = "Model is required";
  if (form.run_mode === "scheduled" && !form.cron_expression.trim())
    errors.cron_expression = "Cron expression is required for scheduled mode";
  return errors;
}

function formToInput(form: FormState): CreateProfileInput {
  return {
    name:                         form.name.trim(),
    description:                  form.description.trim() || null,
    system_prompt:                form.system_prompt,
    provider:                     form.provider as Provider,
    model:                        form.model.trim(),
    context_strategy:             form.context_strategy,
    persistent_memory:            form.persistent_memory,
    max_input_tokens:             form.max_input_tokens === "" ? null : form.max_input_tokens,
    max_output_tokens:            form.max_output_tokens === "" ? null : form.max_output_tokens,
    run_mode:                     form.run_mode,
    cron_expression:              form.cron_expression.trim() || null,
    continuous_poll_interval_secs: form.continuous_poll_interval_secs === "" ? 30 : form.continuous_poll_interval_secs,
    max_iterations:               form.max_iterations === "" ? 20 : form.max_iterations,
    max_retries:                  form.max_retries === "" ? 2 : form.max_retries,
    scope:                        form.scope,
  };
}

// ---------------------------------------------------------------------------
// Provider / mode select options
// ---------------------------------------------------------------------------

const PROVIDER_OPTIONS: SelectOption<Provider>[] = (
  Object.keys(PROVIDER_LABELS) as Provider[]
).map(v => ({ value: v, label: PROVIDER_LABELS[v] }));

const RUN_MODE_OPTIONS: SelectOption<RunMode>[] = [
  { value: "manual",     label: "Manual — run on demand" },
  { value: "continuous", label: "Continuous — auto-pick stories" },
  { value: "scheduled",  label: "Scheduled — run on cron" },
];

// ---------------------------------------------------------------------------
// AgentProfileForm
// ---------------------------------------------------------------------------

interface AgentProfileFormProps {
  /** When non-null: editing. When null: creating. */
  editing: AgentProfile | null;
  open: boolean;
  onClose: () => void;
  onSave: (input: CreateProfileInput) => Promise<void>;
}

export function AgentProfileForm({ editing, open, onClose, onSave }: AgentProfileFormProps) {
  const [form, setForm] = useState<FormState>(DEFAULT_FORM);
  const [errors, setErrors] = useState<FormErrors>({});
  const [saving, setSaving] = useState(false);
  const { activeWorkspace } = useWorkspaceContext();

  // Per-profile permissions (only loaded/saved when editing an existing profile).
  const { permissions: loadedPerms, save: savePerms } = useAgentPermissions(
    editing?.id ?? null
  );
  const [perms, setPerms] = useState<AgentPermissions>(defaultPermissions(""));

  // Tool bindings — only meaningful when editing an existing profile.
  const { servers: allServers } = useMcpServers();
  const { bindings, createBinding, deleteBinding } = useToolBindings(editing?.id ?? "");
  const [bindingServerId, setBindingServerId] = useState("");

  // Servers not yet bound to this profile.
  const boundServerIds = new Set(bindings.map((b) => b.mcp_server_id));
  const availableServers = allServers.filter((s) => !boundServerIds.has(s.id));

  // Custom tool bindings.
  const { tools: allCustomTools } = useCustomTools();
  const { bindings: customBindings, createBinding: createCustomBinding, deleteBinding: deleteCustomBinding } =
    useCustomToolBindings(editing?.id ?? "");
  const [customBindToolId, setCustomBindToolId] = useState("");
  const boundCustomToolIds = new Set(customBindings.map((b) => b.custom_tool_id));
  const availableCustomTools = allCustomTools.filter((t) => !boundCustomToolIds.has(t.id));

  // Reset / populate when panel opens
  useEffect(() => {
    if (open) {
      setForm(editing ? profileToForm(editing) : DEFAULT_FORM);
      setErrors({});
      setPerms(editing ? loadedPerms : defaultPermissions(""));
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, editing]);

  const set = <K extends keyof FormState>(key: K, value: FormState[K]) => {
    setForm(prev => ({ ...prev, [key]: value }));
    // Clear field error on change
    if (errors[key]) setErrors(prev => ({ ...prev, [key]: undefined }));
  };

  const handleSave = async () => {
    const errs = validate(form);
    if (Object.keys(errs).length > 0) {
      setErrors(errs);
      return;
    }
    setSaving(true);
    try {
      await onSave(formToInput(form));
      // Save permissions if we have a profile id (edit mode) or after create.
      if (editing?.id) {
        await savePerms({ ...perms, profileId: editing.id });
        // Sync profile to TOML file whenever scope or content changes.
        await invoke("save_profile_toml", {
          id: editing.id,
          scope: form.scope,
          workspaceRoot: activeWorkspace?.path ?? null,
        });
      }
      onClose();
    } catch (e) {
      setErrors({ name: String(e) });
    } finally {
      setSaving(false);
    }
  };

  // Dynamic model list for selected provider
  const modelOptions: SelectOption[] = form.provider
    ? PROVIDER_MODELS[form.provider]
    : [];
  const ollamaSelected = form.provider === "ollama";

  // When provider changes, clear the model
  const handleProviderChange = (v: Provider | "") => {
    setForm(prev => ({ ...prev, provider: v, model: "" }));
    if (errors.provider) setErrors(prev => ({ ...prev, provider: undefined }));
  };

  const footer = (
    <div style={{ display: "flex", gap: 8 }}>
      <button
        className="btn btn--primary"
        onClick={handleSave}
        disabled={saving}
      >
        {saving ? "Saving…" : editing ? "Save changes" : "Create profile"}
      </button>
      <button className="btn btn--ghost" onClick={onClose} disabled={saving}>
        Cancel
      </button>
    </div>
  );

  return (
    <SlidePanel
      open={open}
      onClose={onClose}
      title={editing ? `Edit: ${editing.name}` : "New Agent Profile"}
      footer={footer}
      width={540}
    >
      <div className="agent-form">
        {/* ── Identity ──────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Identity</h3>

          <FormField label="Name" required error={errors.name}>
            {(id, hasError) => (
              <TextInput
                id={id}
                value={form.name}
                onChange={e => set("name", e.target.value)}
                placeholder="e.g. Research Agent"
                hasError={hasError}
              />
            )}
          </FormField>

          <FormField label="Scope" helperText={form.scope === "workspace" ? `Saved to ${activeWorkspace?.path ?? "workspace"}/.rusty/agents/` : "Saved to ~/.rusty/agents/ (available in all projects)."}>
            {(id) => (
              <select
                id={id}
                className="form-field__input"
                value={form.scope}
                onChange={e => set("scope", e.target.value as "global" | "workspace")}
                disabled={form.scope === "workspace" && !activeWorkspace}
              >
                <option value="global">Global — available everywhere</option>
                <option value="workspace" disabled={!activeWorkspace}>
                  Workspace{activeWorkspace ? ` — ${activeWorkspace.name}` : " (no workspace open)"}
                </option>
              </select>
            )}
          </FormField>

          {editing?.toml_path && (
            <FormField label="TOML file" helperText="Auto-synced on save.">
              {(id) => (
                <TextInput
                  id={id}
                  value={editing.toml_path!}
                  onChange={() => {}}
                  readOnly
                  style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-xs)" }}
                />
              )}
            </FormField>
          )}

          <FormField
            label="Description"
            helperText="What does this agent do? Shown in the list view."
          >
            {(id) => (
              <TextInput
                id={id}
                value={form.description}
                onChange={e => set("description", e.target.value)}
                placeholder="Optional short description"
              />
            )}
          </FormField>
        </section>

        {/* ── Model ─────────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Model</h3>

          <FormField label="Provider" required error={errors.provider}>
            {(id, hasError) => (
              <FormSelect<Provider>
                id={id}
                value={form.provider}
                options={PROVIDER_OPTIONS}
                onChange={handleProviderChange}
                placeholder="Select provider…"
                hasError={hasError}
              />
            )}
          </FormField>

          <FormField label="Model" required error={errors.model}>
            {(id, hasError) =>
              ollamaSelected ? (
                <TextInput
                  id={id}
                  value={form.model}
                  onChange={e => set("model", e.target.value)}
                  placeholder="e.g. llama3:8b"
                  hasError={hasError}
                />
              ) : (
                <FormSelect
                  id={id}
                  value={form.model}
                  options={modelOptions}
                  onChange={v => set("model", v)}
                  placeholder={form.provider ? "Select model…" : "Select a provider first"}
                  disabled={!form.provider}
                  hasError={hasError}
                />
              )
            }
          </FormField>
        </section>

        {/* ── System Prompt ─────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">System Prompt</h3>
          <FormField
            label="System prompt"
            helperText="Markdown supported. Injected as the system message on every run."
          >
            {(id) => (
              <Textarea
                id={id}
                value={form.system_prompt}
                onChange={e => set("system_prompt", e.target.value)}
                mono
                rows={8}
                placeholder="You are a helpful research assistant…"
              />
            )}
          </FormField>
        </section>

        {/* ── Context & Memory ──────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Context &amp; Memory</h3>

          <FormField
            label="Context strategy"
            helperText={
              CONTEXT_STRATEGY_OPTIONS.find(o => o.value === form.context_strategy)?.description
            }
          >
            {(id) => (
              <FormSelect<ContextStrategy>
                id={id}
                value={form.context_strategy}
                options={CONTEXT_STRATEGY_OPTIONS}
                onChange={v => set("context_strategy", (v || "recent") as ContextStrategy)}
              />
            )}
          </FormField>

          <FormField
            label="Persistent memory"
            labelAs="div"
            helperText="Save and recall memories across runs."
          >
            {(id) => (
              <Toggle
                id={id}
                checked={form.persistent_memory}
                onChange={v => set("persistent_memory", v)}
                label="Enable persistent memory"
              />
            )}
          </FormField>
        </section>

        {/* ── Limits ────────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Limits</h3>

          <div className="agent-form__row">
            <FormField
              label="Max input tokens"
              helperText="The context budget for each call. Leave blank to derive one from the model, with room reserved for the response."
            >
              {(id) => (
                <NumberInput
                  id={id}
                  value={form.max_input_tokens}
                  onChange={v => set("max_input_tokens", v)}
                  placeholder="e.g. 100000"
                  min={1}
                />
              )}
            </FormField>

            <FormField label="Max output tokens" helperText="Leave blank for provider default.">
              {(id) => (
                <NumberInput
                  id={id}
                  value={form.max_output_tokens}
                  onChange={v => set("max_output_tokens", v)}
                  placeholder="e.g. 4096"
                  min={1}
                />
              )}
            </FormField>
          </div>

          <FormField label="Max iterations" helperText="Hard cap on agentic loops per run.">
            {(id) => (
              <NumberInput
                id={id}
                value={form.max_iterations}
                onChange={v => set("max_iterations", v)}
                min={1}
                max={200}
              />
            )}
          </FormField>

          <FormField
            label="Max retries"
            helperText="Retries of a failed provider call, within the run — a rate limit costs the wait, not the work already done. 0 disables retrying."
          >
            {(id) => (
              <NumberInput
                id={id}
                value={form.max_retries}
                onChange={v => set("max_retries", v)}
                min={0}
                max={10}
              />
            )}
          </FormField>
        </section>

        {/* ── Run mode ──────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Run Mode</h3>

          <FormField label="Mode">
            {(id) => (
              <FormSelect<RunMode>
                id={id}
                value={form.run_mode}
                options={RUN_MODE_OPTIONS}
                onChange={v => set("run_mode", (v || "manual") as RunMode)}
              />
            )}
          </FormField>

          {form.run_mode === "continuous" && (
            <FormField label="Poll interval (seconds)" helperText="How often to check for new stories.">
              {(id) => (
                <NumberInput
                  id={id}
                  value={form.continuous_poll_interval_secs}
                  onChange={v => set("continuous_poll_interval_secs", v)}
                  min={5}
                  max={3600}
                />
              )}
            </FormField>
          )}

          {form.run_mode === "scheduled" && (
            <FormField
              label="Cron expression"
              required
              error={errors.cron_expression}
              helperText='Standard 5-field cron, e.g. "0 9 * * 1-5" (9am Mon–Fri).'
            >
              {(id, hasError) => (
                <TextInput
                  id={id}
                  value={form.cron_expression}
                  onChange={e => set("cron_expression", e.target.value)}
                  placeholder="0 9 * * 1-5"
                  hasError={hasError}
                  style={{ fontFamily: "var(--font-mono)" }}
                />
              )}
            </FormField>
          )}
        </section>

        {/* ── Tool Access ───────────────────────────────────────────── */}
        {editing && (
          <section className="agent-form__section">
            <h3 className="agent-form__section-title">Tool Access</h3>
            <p className="agent-form__section-hint">
              Connect MCP servers to give this agent access to tools.
            </p>

            {/* Existing bindings */}
            {bindings.length > 0 && (
              <ul className="agent-form__bindings">
                {bindings.map((b) => (
                  <li key={b.id} className="agent-form__binding">
                    <span className="agent-form__binding-name">
                      {b.mcp_server_name ?? b.mcp_server_id}
                    </span>
                    <span className="agent-form__binding-tools">
                      {b.allowed_tools
                        ? `${b.allowed_tools.length} tool${b.allowed_tools.length !== 1 ? "s" : ""}`
                        : "All tools"}
                    </span>
                    <button
                      className="agent-form__binding-remove"
                      onClick={() => deleteBinding(b.id)}
                      aria-label={`Remove binding to ${b.mcp_server_name}`}
                      title="Remove"
                    >
                      <Unlink size={12} />
                    </button>
                  </li>
                ))}
              </ul>
            )}

            {/* Add binding row */}
            {availableServers.length > 0 && (
              <div className="agent-form__bind-row">
                <select
                  className="form-field__input agent-form__bind-select"
                  value={bindingServerId}
                  onChange={(e) => setBindingServerId(e.target.value)}
                >
                  <option value="">— Select a server —</option>
                  {availableServers.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
                </select>
                <button
                  className="btn btn--secondary btn--sm"
                  disabled={!bindingServerId}
                  onClick={async () => {
                    if (!bindingServerId || !editing) return;
                    await createBinding({
                      agent_profile_id: editing.id,
                      mcp_server_id: bindingServerId,
                    });
                    setBindingServerId("");
                  }}
                >
                  Attach
                </button>
              </div>
            )}

            {allServers.length === 0 && (
              <p className="agent-form__bind-empty">
                No MCP servers configured. Add one on the MCP Servers page first.
              </p>
            )}
          </section>
        )}

        {/* ── Custom Shell Tools ────────────────────────────────────── */}
        {editing && (
          <section className="agent-form__section">
            <h3 className="agent-form__section-title">Custom Shell Tools</h3>
            <p className="agent-form__section-hint">
              Give this agent access to pre-defined shell commands.
            </p>

            {customBindings.length > 0 && (
              <ul className="agent-form__bindings">
                {customBindings.map((b) => (
                  <li key={b.custom_tool_id} className="agent-form__binding">
                    <span className="agent-form__binding-name">
                      {b.tool_name ?? b.custom_tool_id}
                    </span>
                    <button
                      className="agent-form__binding-remove"
                      onClick={() => deleteCustomBinding(b.custom_tool_id)}
                      aria-label={`Remove custom tool ${b.tool_name}`}
                      title="Remove"
                    >
                      <Unlink size={12} />
                    </button>
                  </li>
                ))}
              </ul>
            )}

            {availableCustomTools.length > 0 && (
              <div className="agent-form__bind-row">
                <select
                  className="form-field__input agent-form__bind-select"
                  value={customBindToolId}
                  onChange={(e) => setCustomBindToolId(e.target.value)}
                >
                  <option value="">— Select a tool —</option>
                  {availableCustomTools.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.name}
                    </option>
                  ))}
                </select>
                <button
                  className="btn btn--secondary btn--sm"
                  disabled={!customBindToolId}
                  onClick={async () => {
                    if (!customBindToolId) return;
                    await createCustomBinding(customBindToolId);
                    setCustomBindToolId("");
                  }}
                >
                  Attach
                </button>
              </div>
            )}

            {allCustomTools.length === 0 && (
              <p className="agent-form__bind-empty">
                No custom tools defined yet. Add one on the MCP Servers page.
              </p>
            )}
          </section>
        )}

        {/* ── Permissions ───────────────────────────────────────────── */}
        {editing && (
          <section className="agent-form__section">
            <h3 className="agent-form__section-title">Permissions</h3>
            <PermissionEditor value={perms} onChange={setPerms} />
          </section>
        )}
      </div>
    </SlidePanel>
  );
}
