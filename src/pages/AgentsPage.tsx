import { useState } from "react";
import { PageHeader } from "../components/board/PageHeader";
import { AgentList } from "../components/agents/AgentList";
import { AgentProfileForm } from "../components/agents/AgentProfileForm";
import { ConfirmDialog } from "../components/forms";
import { useAgents } from "../hooks/useAgents";
import type { AgentProfile, CreateProfileInput } from "../types/agent";

export default function AgentsPage() {
  const { profiles, loading, error, createProfile, updateProfile, deleteProfile } = useAgents();

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<AgentProfile | null>(null);
  const [deleting, setDeleting] = useState<AgentProfile | null>(null);

  const handleNew = () => {
    setEditing(null);
    setFormOpen(true);
  };

  const handleEdit = (profile: AgentProfile) => {
    setEditing(profile);
    setFormOpen(true);
  };

  const handleSave = async (input: CreateProfileInput) => {
    if (editing) {
      await updateProfile(editing.id, input);
    } else {
      await createProfile(input);
    }
  };

  const handleDeleteConfirm = async () => {
    if (!deleting) return;
    await deleteProfile(deleting.id);
    setDeleting(null);
  };

  return (
    <div className="agents-page">
      <PageHeader title="Agents" ctaLabel="+ New Agent" onCta={handleNew} />

      <div className="agents-page__content">
        {loading && (
          <div className="agents-page__loading">Loading profiles…</div>
        )}
        {error && (
          <div className="agents-page__error">{error}</div>
        )}
        {!loading && !error && (
          <AgentList
            profiles={profiles}
            onEdit={handleEdit}
            onDelete={setDeleting}
          />
        )}
      </div>

      <AgentProfileForm
        open={formOpen}
        editing={editing}
        onClose={() => setFormOpen(false)}
        onSave={handleSave}
      />

      <ConfirmDialog
        open={deleting !== null}
        title="Delete agent profile"
        body={
          deleting
            ? `Are you sure you want to delete "${deleting.name}"? This cannot be undone.`
            : ""
        }
        confirmLabel="Delete"
        onConfirm={handleDeleteConfirm}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}

