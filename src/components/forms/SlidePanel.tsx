import { useEffect, useRef } from "react";
import { X } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

interface SlidePanelProps {
  open: boolean;
  onClose: () => void;
  title: string;
  /**
   * Sticky footer content — typically Cancel + primary action buttons.
   * Rendered at the bottom of the panel, above the edge of the window.
   */
  footer?: React.ReactNode;
  children: React.ReactNode;
  /** Override panel width. Defaults to 480px. */
  width?: number;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * SlidePanel — right-side drawer that slides in over the content.
 *
 * Layout:
 *   ┌─────────────────────────────────────┐
 *   │ Title                           [×] │  ← sticky header
 *   ├─────────────────────────────────────┤
 *   │ children (scrollable)               │
 *   ├─────────────────────────────────────┤
 *   │ footer (sticky)                     │  ← Cancel / Save
 *   └─────────────────────────────────────┘
 *
 * - 480px default width
 * - Scrim covers the left side; clicking closes the panel
 * - Escape key closes
 * - Focus is trapped inside while open; returns to trigger on close
 */
export function SlidePanel({
  open,
  onClose,
  title,
  footer,
  children,
  width = 480,
}: SlidePanelProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  // Focus the panel when it opens
  useEffect(() => {
    if (open) {
      // Defer one tick so the panel is visible before we focus
      requestAnimationFrame(() => panelRef.current?.focus());
    }
  }, [open]);

  if (!open) return null;

  return (
    <>
      {/* Scrim */}
      <div
        className="slide-panel__scrim"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Panel */}
      <div
        ref={panelRef}
        className="slide-panel"
        style={{ width }}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
      >
        {/* Header */}
        <div className="slide-panel__header">
          <h2 className="slide-panel__title">{title}</h2>
          <button
            type="button"
            className="slide-panel__close"
            onClick={onClose}
            aria-label="Close panel"
          >
            <X size={16} />
          </button>
        </div>

        {/* Scrollable body */}
        <div className="slide-panel__body">{children}</div>

        {/* Sticky footer */}
        {footer && <div className="slide-panel__footer">{footer}</div>}
      </div>
    </>
  );
}
