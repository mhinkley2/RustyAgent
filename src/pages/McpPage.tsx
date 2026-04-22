import { useState } from "react";
import { PageHeader } from "../components/board/PageHeader";
import { McpServerList } from "../components/mcp/McpServerList";
import { McpServerForm } from "../components/mcp/McpServerForm";
import { ConfirmDialog } from "../components/forms";
import { useMcpServers } from "../hooks/useMcpServers";
import type { McpServer, CreateMcpServerInput } from "../types/mcp";

export default function McpPage() {
  const { servers, loading, error, createServer, updateServer, deleteServer } =
    useMcpServers();

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [deleting, setDeleting] = useState<McpServer | null>(null);

  const handleNew = () => {
    setEditing(null);
    setFormOpen(true);
  };

  const handleEdit = (server: McpServer) => {
    setEditing(server);
    setFormOpen(true);
  };

  const handleSave = async (input: CreateMcpServerInput) => {
    if (editing) {
      await updateServer(editing.id, input);
    } else {
      await createServer(input);
    }
  };

  const handleDeleteConfirm = async () => {
    if (!deleting) return;
    await deleteServer(deleting.id);
    setDeleting(null);
  };

  return (
    <div className="mcp-page">
      <PageHeader title="MCP Servers" ctaLabel="+ Add Server" onCta={handleNew} />

      <div className="mcp-page__content">
        {loading && (
          <div className="mcp-page__loading">Loading servers…</div>
        )}
        {error && (
          <div className="mcp-page__error">{error}</div>
        )}
        {!loading && !error && (
          <McpServerList
            servers={servers}
            onEdit={handleEdit}
            onDelete={setDeleting}
          />
        )}
      </div>

      <McpServerForm
        open={formOpen}
        editing={editing}
        onClose={() => setFormOpen(false)}
        onSave={handleSave}
      />

      <ConfirmDialog
        open={deleting !== null}
        title="Delete MCP server"
        body={
          deleting
            ? `Are you sure you want to delete "${deleting.name}"? Any agent bindings for this server will also be removed.`
            : ""
        }
        confirmLabel="Delete"
        onConfirm={handleDeleteConfirm}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}
