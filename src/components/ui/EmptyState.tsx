// ─── Types ────────────────────────────────────────────────────────────────────

interface EmptyStateAction {
  label: string;
  onClick: () => void;
}

interface EmptyStateProps {
  /** 48px icon, rendered in --text-muted color. */
  icon?: React.ReactNode;
  heading: string;
  body?: string;
  /** Primary CTA — used for first-time empty states (e.g. "Create Agent"). */
  action?: EmptyStateAction;
  /**
   * When true, renders the filter-caused variant:
   * - heading and body describe the filter result (not first-time empty)
   * - shows "Clear filters" link instead of the primary CTA button
   */
  filtersCaused?: boolean;
  /** Called when the user clicks "Clear filters" (filtersCaused only). */
  onClearFilters?: () => void;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * EmptyState — rendered inside any empty list, table, or board column.
 *
 * Two modes:
 *   1. First-time empty  → icon + heading + body + primary CTA button
 *   2. Filter-caused     → icon + heading + body + "Clear filters" link
 */
export default function EmptyState({
  icon,
  heading,
  body,
  action,
  filtersCaused = false,
  onClearFilters,
}: EmptyStateProps) {
  return (
    <div
      className="empty-state"
      role="status"
      aria-label={heading}
    >
      {icon && (
        <div className="empty-state__icon" aria-hidden="true">
          {icon}
        </div>
      )}

      <h3 className="empty-state__heading">{heading}</h3>

      {body && <p className="empty-state__body">{body}</p>}

      {filtersCaused ? (
        onClearFilters && (
          <button
            className="empty-state__clear"
            onClick={onClearFilters}
          >
            Clear filters
          </button>
        )
      ) : (
        action && (
          <button
            className="empty-state__action"
            onClick={action.onClick}
          >
            {action.label}
          </button>
        )
      )}
    </div>
  );
}
