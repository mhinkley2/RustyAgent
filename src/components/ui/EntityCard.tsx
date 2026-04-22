// ─── Types ────────────────────────────────────────────────────────────────────

export interface EntityCardStat {
  label: string;
  value: string | React.ReactNode;
}

export interface EntityCardAction {
  label: string;
  icon?: React.ReactNode;
  onClick: (e: React.MouseEvent) => void;
  variant?: "default" | "danger";
}

interface EntityCardProps {
  /** 16–20px icon shown in the header next to the title. */
  icon?: React.ReactNode;
  title: string;
  /** Short subtitle line, e.g. "gpt-4o · anthropic · manual". */
  subtitle?: string;
  /** Optional body text, clamped to 2 lines. */
  description?: string;
  /** Status badge or other status indicator. */
  status?: React.ReactNode;
  /** Key stats surfaced in the card footer. */
  stats?: EntityCardStat[];
  /** Actions that appear on hover in the card footer. */
  actions?: EntityCardAction[];
  /** Opens a detail panel — fired when clicking the card body (not actions). */
  onClick?: () => void;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * EntityCard — card representing a single entity (agent, MCP server, etc.).
 *
 * Layout:
 *   [icon] Title              [status badge]
 *   subtitle line
 *   ─────────────────────────────────────────
 *   description (2 lines max)
 *   ─────────────────────────────────────────
 *   stat 1 │ stat 2 │ stat 3    [Edit] [Run]   ← actions on hover
 */
export default function EntityCard({
  icon,
  title,
  subtitle,
  description,
  status,
  stats,
  actions,
  onClick,
}: EntityCardProps) {
  return (
    <article
      className={onClick ? "entity-card entity-card--clickable" : "entity-card"}
      onClick={onClick}
    >
      {/* Header */}
      <div className="entity-card__header">
        <div className="entity-card__title-group">
          {icon && (
            <span className="entity-card__icon" aria-hidden="true">
              {icon}
            </span>
          )}
          <span className="entity-card__title">{title}</span>
        </div>
        {status && <div className="entity-card__status">{status}</div>}
      </div>

      {subtitle && (
        <p className="entity-card__subtitle">{subtitle}</p>
      )}

      {/* Optional body */}
      {description && (
        <p className="entity-card__description">{description}</p>
      )}

      {/* Footer: stats + hover actions */}
      {(stats || actions) && (
        <div className="entity-card__footer">
          {stats && stats.length > 0 && (
            <dl className="entity-card__stats">
              {stats.map((stat, i) => (
                <div key={i} className="entity-card__stat">
                  <dt className="entity-card__stat-label">{stat.label}</dt>
                  <dd className="entity-card__stat-value">{stat.value}</dd>
                </div>
              ))}
            </dl>
          )}

          {actions && actions.length > 0 && (
            <div
              className="entity-card__actions"
              onClick={(e) => e.stopPropagation()}
            >
              {actions.map((action, i) => (
                <button
                  key={i}
                  className={
                    action.variant === "danger"
                      ? "entity-card__action entity-card__action--danger"
                      : "entity-card__action"
                  }
                  onClick={action.onClick}
                  title={action.label}
                  aria-label={action.label}
                >
                  {action.icon && (
                    <span aria-hidden="true">{action.icon}</span>
                  )}
                  <span>{action.label}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </article>
  );
}
