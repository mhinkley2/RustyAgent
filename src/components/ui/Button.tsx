import { Loader2 } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type ButtonVariant = "primary" | "secondary" | "ghost" | "destructive" | "link";
export type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Show spinner and disable while true. */
  loading?: boolean;
  /** Label shown while loading (replaces children). */
  loadingLabel?: string;
  /** Icon placed before the label. */
  icon?: React.ReactNode;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * Button — full button system with 5 variants and 3 sizes.
 *
 * Loading state: spinner replaces icon, `loadingLabel` (or "Working…")
 * replaces children, button is disabled. Never silently disabled.
 */
export function Button({
  variant = "primary",
  size = "md",
  loading = false,
  loadingLabel,
  icon,
  children,
  className,
  disabled,
  ...rest
}: ButtonProps) {
  const classes = [
    "btn",
    `btn--${variant}`,
    size !== "md" ? `btn--${size}` : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      className={classes}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? (
        <>
          <Loader2 size={size === "sm" ? 12 : size === "lg" ? 18 : 14} className="btn__spinner" aria-hidden="true" />
          {loadingLabel ?? "Working…"}
        </>
      ) : (
        <>
          {icon && <span className="btn__icon" aria-hidden="true">{icon}</span>}
          {children}
        </>
      )}
    </button>
  );
}

// ─── IconButton ────────────────────────────────────────────────────────────────

interface IconButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** aria-label is required — icon buttons must always have a label. */
  "aria-label": string;
  icon: React.ReactNode;
  variant?: "ghost" | "secondary";
  size?: "sm" | "md";
  loading?: boolean;
}

/**
 * IconButton — square button for icon-only actions.
 * aria-label is required; a Tooltip wrapping this provides the visible hint.
 */
export function IconButton({
  icon,
  variant = "ghost",
  size = "md",
  loading = false,
  className,
  disabled,
  ...rest
}: IconButtonProps) {
  const classes = [
    "icon-btn",
    `icon-btn--${variant}`,
    size === "sm" ? "icon-btn--sm" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      className={classes}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? (
        <Loader2 size={size === "sm" ? 12 : 15} className="btn__spinner" aria-hidden="true" />
      ) : (
        <span aria-hidden="true">{icon}</span>
      )}
    </button>
  );
}
