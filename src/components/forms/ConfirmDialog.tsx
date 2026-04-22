import { useEffect, useRef } from "react";

// ─── Types ────────────────────────────────────────────────────────────────────

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  /** Describe the exact consequence of the action. */
  body: React.ReactNode;
  /** Label for the destructive confirm button, e.g. "Delete Agent". */
  confirmLabel: string;
  /** Label for the cancel button. Defaults to "Cancel". */
  cancelLabel?: string;
  /** Whether the confirm action is in flight (disables buttons, shows spinner). */
  loading?: boolean;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * ConfirmDialog — centered modal for irreversible/destructive actions.
 *
 * Rules:
 *  - Cancel is auto-focused (never the destructive button)
 *  - Escape closes the dialog
 *  - Clicking the backdrop closes it
 *  - The confirm button is always styled as destructive (red)
 */
export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  body,
  confirmLabel,
  cancelLabel = "Cancel",
  loading = false,
}: ConfirmDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Auto-focus Cancel (safe default)
  useEffect(() => {
    if (open) {
      requestAnimationFrame(() => cancelRef.current?.focus());
    }
  }, [open]);

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

  if (!open) return null;

  return (
    <>
      {/* Backdrop */}
      <div
        className="dialog-backdrop"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Dialog */}
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        aria-describedby="confirm-dialog-body"
      >
        <h2 id="confirm-dialog-title" className="confirm-dialog__title">
          {title}
        </h2>

        <div id="confirm-dialog-body" className="confirm-dialog__body">
          {body}
        </div>

        <div className="confirm-dialog__actions">
          <button
            ref={cancelRef}
            type="button"
            className="btn btn--secondary"
            onClick={onClose}
            disabled={loading}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className="btn btn--destructive"
            onClick={onConfirm}
            disabled={loading}
            aria-busy={loading}
          >
            {loading ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </>
  );
}
