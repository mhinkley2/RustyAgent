import { useState } from "react";
import { PageHeader } from "../components/board/PageHeader";
import { McpServerList } from "../components/mcp/McpServerList";
import { McpServerForm } from "../components/mcp/McpServerForm";
import { CustomToolList } from "../components/mcp/CustomToolList";
import { CustomToolForm } from "../components/mcp/CustomToolForm";
import { ConfirmDialog } from "../components/forms";
import { useMcpServers } from "../hooks/useMcpServers";
import { useCustomTools } from "../hooks/useCustomTools";
import type { McpServer, CreateMcpServerInput } from "../types/mcp";
import type { CustomTool, CreateCustomToolInput } from "../types/custom_tools";

type Tab = "mcp" | "shell";

export default function ToolsPage() {
  const [tab, setTab] = useState<Tab>("mcp");

  // ── MCP servers ────────────────────────────────────────────────────────────
  const { servers, loading: mcpLoading, error: mcpError, createServer, updateServer, deleteServer } =
    useMcpServers();
  const [mcpFormOpen, setMcpFormOpen] = useState(false);
  const [editingMcp, setEditingMcp] = useState<McpServer | null>(null);
  const [deletingMcp, setDeletingMcp] = useState<McpServer | null>(null);

  const handleMcpNew = () => { setEditingMcp(null); setMcpFormOpen(true); };
  const handleMcpEdit = (s: McpServer) => { setEditingMcp(s); setMcpFormOpen(true); };
  const handleMcpSave = async (input: CreateMcpServerInput) => {
    if (editingMcp) await updateServer(editingMcp.id, input);
    else            await createServer(input);
  };
  const handleMcpDeleteConfirm = async () => {
    if (!deletingMcp) return;
    await deleteServer(deletingMcp.id);
    setDeletingMcp(null);
  };

  // ── Custom shell tools ─────────────────────────────────────────────────────
  const { tools, loading: toolsLoading, error: toolsError, createTool, updateTool, deleteTool } =
    useCustomTools();
  const [toolFormOpen, setToolFormOpen] = useState(false);
  const [editingTool, setEditingTool] = useState<CustomTool | null>(null);
  const [deletingTool, setDeletingTool] = useState<CustomTool | null>(null);

  const handleToolNew = () => { setEditingTool(null); setToolFormOpen(true); };
  const handleToolEdit = (t: CustomTool) => { setEditingTool(t); setToolFormOpen(true); };
  const handleToolSave = async (input: CreateCustomToolInput) => {
    if (editingTool) await updateTool(editingTool.id, input);
    else             await createTool(input);
  };
  const handleToolDeleteConfirm = async () => {
    if (!deletingTool) return;
    await deleteTool(deletingTool.id);
    setDeletingTool(null);
  };

  const ctaLabel = tab === "mcp" ? "+ Add Server" : "+ Add Tool";
  const handleCta = tab === "mcp" ? handleMcpNew : handleToolNew;

  return (
    <div className="mcp-page">
      <PageHeader title="Tools" ctaLabel={ctaLabel} onCta={handleCta} />

      {/* Tabs */}
      <div className="tools-tabs">
        <button
          className={`tools-tab${tab === "mcp" ? " tools-tab--active" : ""}`}
          onClick={() => setTab("mcp")}
        >
          MCP Servers
        </button>
        <button
          className={`tools-tab${tab === "shell" ? " tools-tab--active" : ""}`}
          onClick={() => setTab("shell")}
        >
          Shell Commands
        </button>
      </div>

      {/* Tab content */}
      <div className="mcp-page__content">
        {tab === "mcp" && (
          <>
            {mcpLoading && <div className="mcp-page__loading">Loading servers…</div>}
            {mcpError   && <div className="mcp-page__error">{mcpError}</div>}
            {!mcpLoading && !mcpError && (
              <McpServerList
                servers={servers}
                onEdit={handleMcpEdit}
                onDelete={setDeletingMcp}
              />
            )}
          </>
        )}

        {tab === "shell" && (
          <>
            {toolsLoading && <div className="mcp-page__loading">Loading tools…</div>}
            {toolsError   && <div className="mcp-page__error">{toolsError}</div>}
            {!toolsLoading && !toolsError && (
              <CustomToolList
                tools={tools}
                onEdit={handleToolEdit}
                onDelete={setDeletingTool}
              />
            )}
          </>
        )}
      </div>

      {/* MCP panels */}
      <McpServerForm
        open={mcpFormOpen}
        editing={editingMcp}
        onClose={() => setMcpFormOpen(false)}
        onSave={handleMcpSave}
      />
      <ConfirmDialog
        open={deletingMcp !== null}
        title="Delete MCP server"
        body={deletingMcp ? `Delete "${deletingMcp.name}"? Agent bindings will also be removed.` : ""}
        confirmLabel="Delete"
        onConfirm={handleMcpDeleteConfirm}
        onClose={() => setDeletingMcp(null)}
      />

      {/* Shell tool panels */}
      <CustomToolForm
        open={toolFormOpen}
        editing={editingTool}
        onClose={() => setToolFormOpen(false)}
        onSave={handleToolSave}
      />
      <ConfirmDialog
        open={deletingTool !== null}
        title="Delete shell tool"
        body={deletingTool ? `Delete "${deletingTool.name}"? Agent bindings will also be removed.` : ""}
        confirmLabel="Delete"
        onConfirm={handleToolDeleteConfirm}
        onClose={() => setDeletingTool(null)}
      />
    </div>
  );
}
