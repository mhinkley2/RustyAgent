// ─── SkeletonLine ─────────────────────────────────────────────────────────────

interface SkeletonLineProps {
  /** CSS width value, e.g. "60%", "120px". Defaults to "100%". */
  width?: string;
  /** CSS height value. Defaults to "1em". */
  height?: string;
}

/**
 * SkeletonLine — a single animated placeholder bar.
 * Use these to compose page-specific skeleton layouts.
 */
export function SkeletonLine({ width = "100%", height = "1em" }: SkeletonLineProps) {
  return (
    <span
      className="skeleton-line"
      style={{ width, height }}
      aria-hidden="true"
    />
  );
}

// ─── SkeletonCard ─────────────────────────────────────────────────────────────

/**
 * SkeletonCard — placeholder matching the shape of an EntityCard.
 * Shows 3 lines of different widths to simulate header, subtitle, and body.
 */
export function SkeletonCard() {
  return (
    <div className="skeleton-card" aria-hidden="true">
      {/* Header row */}
      <div className="skeleton-card__header">
        <span className="skeleton-avatar" />
        <SkeletonLine width="55%" height="14px" />
      </div>
      {/* Subtitle */}
      <SkeletonLine width="40%" height="12px" />
      {/* Body */}
      <div className="skeleton-card__body">
        <SkeletonLine width="100%" height="12px" />
        <SkeletonLine width="80%" height="12px" />
      </div>
      {/* Footer */}
      <div className="skeleton-card__footer">
        <SkeletonLine width="25%" height="11px" />
        <SkeletonLine width="25%" height="11px" />
        <SkeletonLine width="25%" height="11px" />
      </div>
    </div>
  );
}

// ─── SkeletonTable ────────────────────────────────────────────────────────────

interface SkeletonTableProps {
  /** Number of placeholder rows to render. Defaults to 5. */
  rows?: number;
  /** Number of columns per row. Defaults to 4. */
  cols?: number;
}

/**
 * SkeletonTable — placeholder matching the shape of a data table.
 */
export function SkeletonTable({ rows = 5, cols = 4 }: SkeletonTableProps) {
  return (
    <div className="skeleton-table" aria-hidden="true">
      {/* Header */}
      <div className="skeleton-table__header">
        {Array.from({ length: cols }).map((_, i) => (
          <SkeletonLine key={i} width={i === 0 ? "35%" : "20%"} height="12px" />
        ))}
      </div>
      {/* Rows */}
      {Array.from({ length: rows }).map((_, ri) => (
        <div key={ri} className="skeleton-table__row">
          {Array.from({ length: cols }).map((_, ci) => (
            <SkeletonLine
              key={ci}
              width={ci === 0 ? `${45 + (ri % 3) * 10}%` : "20%"}
              height="13px"
            />
          ))}
        </div>
      ))}
    </div>
  );
}
