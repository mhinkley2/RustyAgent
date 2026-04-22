import type { ReactNode } from "react";

// ---------------------------------------------------------------------------
// PageHeader — shared top bar used by Board, Agents, Runs, MCP, Settings pages
// ---------------------------------------------------------------------------

interface PageHeaderProps {
  title: string;
  /** Right-side primary action button label */
  ctaLabel?: string;
  onCta?: () => void;
  /** Custom right-side element (overrides ctaLabel/onCta when provided) */
  cta?: ReactNode;
  /** Optional content rendered between the title row and edges (e.g. FilterBar) */
  children?: ReactNode;
  /** Make the header stick to the top of its scroll container */
  sticky?: boolean;
}

export function PageHeader({ title, ctaLabel, onCta, cta, children, sticky }: PageHeaderProps) {
  return (
    <div className={`page-header${sticky ? " page-header--sticky" : ""}`}>
      <div className="page-header__row">
        <h1 className="page-header__title">{title}</h1>
        {cta ?? (ctaLabel && (
          <button className="btn btn--primary btn--sm" onClick={onCta}>
            {ctaLabel}
          </button>
        ))}
      </div>
      {children && <div className="page-header__sub">{children}</div>}
    </div>
  );
}
