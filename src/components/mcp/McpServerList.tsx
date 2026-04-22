import { Cpu, Pencil, Trash2 } from "lucide-react";
import type { McpServer } from "../../types/mcp";

// ---------------------------------------------------------------------------
// McpServerRow — a table row for one server
// ---------------------------------------------------------------------------

interface McpServerRowProps {
  server: McpServer;
  onEdit: (server: McpServer) => void;
  onDelete: (server: McpServer) => void;
}

function McpServerRow({ server, onEdit, onDelete }: McpServerRowProps) {
  return (
    <tr className="mcp-table__row">
      <td className="mcp-table__cell mcp-table__cell--name">
        <span className="mcp-table__server-name">{server.name}</span>
      </td>
      <td className="mcp-table__cell mcp-table__cell--command">
        <code className="mcp-table__command">{server.command}</code>
        {server.args.length > 0 && (
          <span className="mcp-table__args">
            {server.args.map((a, i) => (
              <code key={i} className="mcp-table__arg">{a}</code>
            ))}
          </span>
        )}
      </td>
      <td className="mcp-table__cell mcp-table__cell--env">
        {Object.keys(server.env_vars).length > 0 ? (
          <span className="mcp-table__env-count">
            {Object.keys(server.env_vars).length} var
            {Object.keys(server.env_vars).length !== 1 ? "s" : ""}
          </span>
        ) : (
          <span className="mcp-table__env-none">—</span>
        )}
      </td>
      <td className="mcp-table__cell mcp-table__cell--restart">
        {server.auto_restart ? (
          <span className="mcp-badge mcp-badge--on">
            Auto · {server.max_restart_attempts}×
          </span>
        ) : (
          <span className="mcp-badge mcp-badge--off">Off</span>
        )}
      </td>
      <td className="mcp-table__cell mcp-table__cell--actions">
        <button
          className="mcp-table__action-btn"
          onClick={() => onEdit(server)}
          aria-label={`Edit ${server.name}`}
          title="Edit"
        >
          <Pencil size={14} />
        </button>
        <button
          className="mcp-table__action-btn mcp-table__action-btn--danger"
          onClick={() => onDelete(server)}
          aria-label={`Delete ${server.name}`}
          title="Delete"
        >
          <Trash2 size={14} />
        </button>
      </td>
    </tr>
  );
}

// ---------------------------------------------------------------------------
// McpServerList
// ---------------------------------------------------------------------------

interface McpServerListProps {
  servers: McpServer[];
  onEdit: (server: McpServer) => void;
  onDelete: (server: McpServer) => void;
}

export function McpServerList({ servers, onEdit, onDelete }: McpServerListProps) {
  if (servers.length === 0) {
    return (
      <div className="empty-state">
        <Cpu size={40} className="empty-state__icon" />
        <p className="empty-state__title">No MCP servers configured</p>
        <p className="empty-state__body">
          Add a Model Context Protocol server to give agents access to external
          tools — filesystems, databases, APIs, and more.
        </p>
      </div>
    );
  }

  return (
    <div className="mcp-table-wrap">
      <table className="mcp-table">
        <thead>
          <tr>
            <th className="mcp-table__th">Name</th>
            <th className="mcp-table__th">Command / Args</th>
            <th className="mcp-table__th">Env vars</th>
            <th className="mcp-table__th">Restart</th>
            <th className="mcp-table__th mcp-table__th--actions" />
          </tr>
        </thead>
        <tbody>
          {servers.map((server) => (
            <McpServerRow
              key={server.id}
              server={server}
              onEdit={onEdit}
              onDelete={onDelete}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}
