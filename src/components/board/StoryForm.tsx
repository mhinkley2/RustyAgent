import { useState, useEffect } from "react";
import {
  SlidePanel,
  FormField,
  TextInput,
  Textarea,
  FormSelect,
  Toggle,
} from "../forms";
import type { SelectOption } from "../forms";
import type { Story } from "../../types/board";
import type { AgentProfile } from "../../types/agent";
import type { CreateStoryInput, UpdateStoryInput } from "../../hooks/useStories";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type StoryTypeOption = "task" | "human" | "pipeline";
type StoryStatusOption = "backlog" | "ready" | "in_progress" | "blocked" | "review" | "done";
type StoryPriorityOption = "low" | "medium" | "high" | "critical";

interface FormState {
  title: string;
  description: string;
  story_type: StoryTypeOption;
  status: StoryStatusOption;
  priority: StoryPriorityOption;
  assigned_agent_id: string;
  requires_approval: boolean;
  track_history: boolean;
  labels: string;             // comma-separated input
}

type FormErrors = Partial<Record<keyof FormState, string>>;

const DEFAULT_FORM: FormState = {
  title: "",
  description: "",
  story_type: "task",
  status: "backlog",
  priority: "medium",
  assigned_agent_id: "",
  requires_approval: false,
  track_history: true,
  labels: "",
};

function storyToForm(s: Story): FormState {
  return {
    title:             s.title,
    description:       s.description ?? "",
    story_type:        s.type as StoryTypeOption,
    status:            s.status as StoryStatusOption,
    priority:          s.priority as StoryPriorityOption,
    assigned_agent_id: s.assignedAgentId ?? "",
    requires_approval: s.requiresApproval,
    track_history:     s.trackHistory ?? true,
    labels:            s.labels.join(", "),
  };
}

function validate(form: FormState): FormErrors {
  const errors: FormErrors = {};
  if (!form.title.trim()) errors.title = "Title is required.";
  return errors;
}

function labelsFromString(s: string): string[] {
  return s
    .split(",")
    .map(l => l.trim())
    .filter(Boolean);
}

function formToCreate(form: FormState): CreateStoryInput {
  return {
    title:             form.title.trim(),
    description:       form.description.trim() || null,
    story_type:        form.story_type,
    status:            form.status,
    priority:          form.priority,
    assigned_agent_id: form.assigned_agent_id || undefined,
    requires_approval: form.requires_approval,
    track_history:     form.track_history,
    labels:            labelsFromString(form.labels),
  };
}

function formToUpdate(form: FormState): UpdateStoryInput {
  return {
    title:             form.title.trim(),
    description:       form.description.trim() || null,
    story_type:        form.story_type,
    status:            form.status,
    priority:          form.priority,
    // Empty string clears the assignee in the backend.
    assigned_agent_id: form.assigned_agent_id,
    requires_approval: form.requires_approval,
    track_history:     form.track_history,
    labels:            labelsFromString(form.labels),
  };
}

// ---------------------------------------------------------------------------
// Select options
// ---------------------------------------------------------------------------

const TYPE_OPTIONS: SelectOption<StoryTypeOption>[] = [
  { value: "task",     label: "Task" },
  { value: "human",    label: "Human Input" },
  { value: "pipeline", label: "Pipeline" },
];

const STATUS_OPTIONS: SelectOption<StoryStatusOption>[] = [
  { value: "backlog",     label: "Backlog" },
  { value: "ready",       label: "Ready" },
  { value: "in_progress", label: "In Progress" },
  { value: "blocked",     label: "Blocked" },
  { value: "review",      label: "Review" },
  { value: "done",        label: "Done" },
];

const PRIORITY_OPTIONS: SelectOption<StoryPriorityOption>[] = [
  { value: "low",      label: "Low" },
  { value: "medium",   label: "Medium" },
  { value: "high",     label: "High" },
  { value: "critical", label: "Critical" },
];

// ---------------------------------------------------------------------------
// StoryForm
// ---------------------------------------------------------------------------

interface StoryFormProps {
  open: boolean;
  story?: Story | null;        // null/undefined → create mode
  agents: AgentProfile[];
  onClose: () => void;
  onCreate?: (input: CreateStoryInput) => Promise<Story>;
  onUpdate?: (id: string, input: UpdateStoryInput) => Promise<Story>;
}

export function StoryForm({ open, story, agents, onClose, onCreate, onUpdate }: StoryFormProps) {
  const isEdit = story != null;

  const [form, setForm] = useState<FormState>(DEFAULT_FORM);
  const [errors, setErrors] = useState<FormErrors>({});
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Reset form whenever the panel opens or the target story changes.
  useEffect(() => {
    if (open) {
      setForm(story ? storyToForm(story) : DEFAULT_FORM);
      setErrors({});
      setSaveError(null);
    }
  }, [open, story]);

  function set<K extends keyof FormState>(key: K, value: FormState[K]) {
    setForm(prev => ({ ...prev, [key]: value }));
    setErrors(prev => ({ ...prev, [key]: undefined }));
  }

  async function handleSubmit() {
    const errs = validate(form);
    if (Object.keys(errs).length > 0) {
      setErrors(errs);
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      if (isEdit && story && onUpdate) {
        await onUpdate(story.id, formToUpdate(form));
      } else if (!isEdit && onCreate) {
        await onCreate(formToCreate(form));
      }
      onClose();
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  // Build agent options for the assignee dropdown.
  const agentOptions: SelectOption<string>[] = [
    { value: "", label: "Unassigned" },
    ...agents.map(a => ({ value: a.id, label: a.name })),
  ];

  const title = isEdit ? `Edit Story` : "New Story";

  const footer = (
    <div className="form-footer">
      <button className="btn btn--secondary" onClick={onClose} disabled={saving}>
        Cancel
      </button>
      <button className="btn btn--primary" onClick={handleSubmit} disabled={saving}>
        {saving ? "Saving…" : isEdit ? "Save Story" : "Create Story"}
      </button>
    </div>
  );

  return (
    <SlidePanel open={open} onClose={onClose} title={title} footer={footer} width={520}>
      <div className="agent-form">
        {saveError && (
          <div className="agent-form__section">
            <p className="form-error-banner">{saveError}</p>
          </div>
        )}

        {/* ── Core ─────────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Story</h3>

          <FormField label="Title" required error={errors.title}>
            {(id, hasError) => (
              <TextInput
                id={id}
                value={form.title}
                onChange={e => set("title", e.target.value)}
                hasError={hasError}
                placeholder="What needs to be done?"
                autoFocus
              />
            )}
          </FormField>

          <FormField label="Description">
            {(id) => (
              <Textarea
                id={id}
                value={form.description}
                onChange={e => set("description", e.target.value)}
                placeholder="Optional context, acceptance criteria, or notes…"
                rows={4}
              />
            )}
          </FormField>
        </section>

        {/* ── Classification ────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Classification</h3>

          <FormField label="Type">
            {(id) => (
              <FormSelect
                id={id}
                value={form.story_type}
                options={TYPE_OPTIONS}
                onChange={v => v && set("story_type", v as StoryTypeOption)}
              />
            )}
          </FormField>

          <FormField label="Status">
            {(id) => (
              <FormSelect
                id={id}
                value={form.status}
                options={STATUS_OPTIONS}
                onChange={v => v && set("status", v as StoryStatusOption)}
              />
            )}
          </FormField>

          <FormField label="Priority">
            {(id) => (
              <FormSelect
                id={id}
                value={form.priority}
                options={PRIORITY_OPTIONS}
                onChange={v => v && set("priority", v as StoryPriorityOption)}
              />
            )}
          </FormField>
        </section>

        {/* ── Assignment ────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Assignment</h3>

          <FormField label="Assigned Agent">
            {(id) => (
              <FormSelect
                id={id}
                value={form.assigned_agent_id}
                options={agentOptions}
                onChange={v => set("assigned_agent_id", v)}
              />
            )}
          </FormField>

          <FormField
            label="Requires Approval"
            labelAs="div"
            helperText="Agent will pause and wait for human approval before proceeding."
          >
            {(id) => (
              <Toggle
                id={id}
                checked={form.requires_approval}
                onChange={v => set("requires_approval", v)}
                label="Require human approval"
              />
            )}
          </FormField>
        </section>

        {/* ── History ──────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">History</h3>

          <FormField
            label="Track run history"
            labelAs="div"
            helperText="Store all run events (messages, tool calls, results). Disable to suppress verbose history for automated sub-tasks."
          >
            {(id) => (
              <Toggle
                id={id}
                checked={form.track_history}
                onChange={v => set("track_history", v)}
                label="Enable full event history"
              />
            )}
          </FormField>
        </section>

        {/* ── Labels ───────────────────────────────────────────────── */}
        <section className="agent-form__section">
          <h3 className="agent-form__section-title">Labels</h3>

          <FormField label="Labels" helperText="Comma-separated tags, e.g. phase-1, research, urgent">
            {(id) => (
              <TextInput
                id={id}
                value={form.labels}
                onChange={e => set("labels", e.target.value)}
                placeholder="phase-1, research"
              />
            )}
          </FormField>
        </section>
      </div>
    </SlidePanel>
  );
}
