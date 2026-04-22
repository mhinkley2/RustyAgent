import { Pencil, Terminal, Trash2 } from "lucide-react";
import type { CustomTool } from "../../types/custom_tools";

interface CustomToolRowProps {
  tool: CustomTool;
  onEdit: (tool: CustomTool) => void;
  onDelete: (tool: CustomTool) => void;
}

function CustomToolRow({ tool, onEdit, onDelete }: CustomToolRowProps) {
  return (
    <tr className="mcp-table__row">
      <td className="mcp-table__cell mcp-table__cell--name">
        <span className="mcp-table__server-name">
          <Terminal size={12} style={{ display: "inline", marginRight: 4, opacity: 0.6 }} />
          {tool.name}
        </span>
        {tool.description && (
          <span className="mcp-table__args" style={{ display: "block", fontSize: "var(--text-xs)", color: "var(--text-secondary)", marginTop: 2 }}>
            {tool.description}
          </span>
        )}
      </td>
      <td className="mcp-table__cell mcp-table__cell--command">
        <code className="mcp-table__command">{tool.command}</code>
        {tool.working_dir !== "." && (
          <span className="mcp-table__args">
            in <code className="mcp-table__arg">{tool.working_dir}</code>
          </span>
        )}
      </td>
      <td className="mcp-table__cell">
        <span className="mcp-badge mcp-badge--on">
          {tool.timeout_secs}s
        </span>
      </td>
      <td className="mcp-table__cell mcp-table__cell--actions">
        <button
          className="mcp-table__action-btn"
          onClick={() => onEdit(tool)}
          aria-label={`Edit ${tool.name}`}
          title="Edit"
        >
          <Pencil size={14} />
        </button>
        <button
          className="mcp-table__action-btn mcp-table__action-btn--danger"
          onClick={() => onDelete(tool)}
          aria-label={`Delete ${tool.name}`}
          title="Delete"
        >
          <Trash2 size={14} />
        </button>
      </td>
    </tr>
  );
}

interface CustomToolListProps {
  tools: CustomTool[];
  onEdit: (tool: CustomTool) => void;
  onDelete: (tool: CustomTool) => void;
}

export function CustomToolList({ tools, onEdit, onDelete }: CustomToolListProps) {
  if (tools.length === 0) {
    return (
      <p className="mcp-empty">No custom tools defined yet. Add one to give agents runnable commands.</p>
    );
  }

  return (
    <table className="mcp-table">
      <thead>
        <tr>
          <th className="mcp-table__header">Tool name</th>
          <th className="mcp-table__header">Command</th>
          <th className="mcp-table__header">Timeout</th>
          <th className="mcp-table__header mcp-table__header--actions">Actions</th>
        </tr>
      </thead>
      <tbody>
        {tools.map((tool) => (
          <CustomToolRow key={tool.id} tool={tool} onEdit={onEdit} onDelete={onDelete} />
        ))}
      </tbody>
    </table>
  );
}
