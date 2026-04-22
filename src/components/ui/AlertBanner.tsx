import { AlertTriangle, Info, X, AlertCircle } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type AlertVariant = "info" | "warning" | "error";

interface AlertBannerProps {
  variant: AlertVariant;
  message: React.ReactNode;
  /** Optional inline CTA link. */
  action?: { label: string; onClick: () => void };
  /** When provided, the banner has an × dismiss button. */
  onDismiss?: () => void;
}

// ─── Component ────────────────────────────────────────────────────────────────

const ICONS: Record<AlertVariant, React.ReactNode> = {
  info:    <Info size={15} />,
  warning: <AlertTriangle size={15} />,
  error:   <AlertCircle size={15} />,
};

/**
 * AlertBanner — full-width contextual message below the page header.
 * One per page maximum. Disappears when the condition clears (parent controls visibility).
 */
export function AlertBanner({ variant, message, action, onDismiss }: AlertBannerProps) {
  return (
    <div
      className={`alert-banner alert-banner--${variant}`}
      role={variant === "error" ? "alert" : "status"}
    >
      <span className="alert-banner__icon" aria-hidden="true">
        {ICONS[variant]}
      </span>

      <span className="alert-banner__message">{message}</span>

      {action && (
        <button
          type="button"
          className="alert-banner__action"
          onClick={action.onClick}
        >
          {action.label}
        </button>
      )}

      {onDismiss && (
        <button
          type="button"
          className="alert-banner__close"
          onClick={onDismiss}
          aria-label="Dismiss"
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
}
