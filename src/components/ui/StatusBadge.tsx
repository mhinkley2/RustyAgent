import { Loader2, Check, Clock, Pause, X, Lock, Minus, ArrowRight } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type StatusVariant =
  | "running"
  | "done"
  | "scheduled"
  | "idle"
  | "failed"
  | "blocked"
  | "backlog"
  | "in-progress"
  | "ready";

interface StatusConfig {
  label: string;
  getIcon: (size: number) => React.ReactNode;
  modifier: string;
  /** Whether the icon should spin (running / in-progress). */
  spin?: boolean;
}

const STATUS_CONFIG: Record<StatusVariant, StatusConfig> = {
  running:       { label: "Running",     getIcon: (s) => <Loader2 size={s} />,    modifier: "running",     spin: true  },
  done:          { label: "Done",        getIcon: (s) => <Check size={s} />,       modifier: "done" },
  scheduled:     { label: "Scheduled",   getIcon: (s) => <Clock size={s} />,       modifier: "scheduled" },
  idle:          { label: "Idle",        getIcon: (s) => <Pause size={s} />,       modifier: "idle" },
  failed:        { label: "Failed",      getIcon: (s) => <X size={s} />,           modifier: "failed" },
  blocked:       { label: "Blocked",     getIcon: (s) => <Lock size={s} />,        modifier: "blocked" },
  backlog:       { label: "Backlog",     getIcon: (s) => <Minus size={s} />,       modifier: "backlog" },
  "in-progress": { label: "In Progress", getIcon: (s) => <Loader2 size={s} />,    modifier: "in-progress", spin: true  },
  ready:         { label: "Ready",       getIcon: (s) => <ArrowRight size={s} />,  modifier: "ready" },
};

// ─── Component ─────────────────────────────────────────────────────────────────

interface StatusBadgeProps {
  status: StatusVariant;
  size?: "sm" | "md";
  className?: string;
}

/**
 * StatusBadge — pill with icon + label.
 * Always pairs an icon AND text with color so it's never color-only.
 */
export default function StatusBadge({ status, size = "md", className }: StatusBadgeProps) {
  const config = STATUS_CONFIG[status];
  const iconSize = size === "sm" ? 11 : 13;

  const classes = [
    "status-badge",
    `status-badge--${config.modifier}`,
    size === "sm" ? "status-badge--sm" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span className={classes} aria-label={config.label}>
      <span
        className={
          config.spin
            ? "status-badge__icon status-badge__icon--spin"
            : "status-badge__icon"
        }
        aria-hidden="true"
      >
        {config.getIcon(iconSize)}
      </span>
      <span className="status-badge__label">{config.label}</span>
    </span>
  );
}
