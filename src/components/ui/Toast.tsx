import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { Check, X, AlertTriangle, Info } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type ToastVariant = "success" | "error" | "warning" | "info";

export interface Toast {
  id: string;
  variant: ToastVariant;
  title: string;
  body?: string;
  /** Optional CTA shown as a link button inside the toast. */
  action?: { label: string; onClick: () => void };
  /**
   * Auto-dismiss after this many ms. Defaults to 4000 for non-errors.
   * Set to 0 to never auto-dismiss.
   */
  duration?: number;
}

export type ToastInput = Omit<Toast, "id">;

interface ToastContextValue {
  show: (toast: ToastInput) => void;
  dismiss: (id: string) => void;
}

const pendingToasts: ToastInput[] = [];
let globalShowToast: ((toast: ToastInput) => void) | null = null;

export function notifyToast(toast: ToastInput) {
  if (globalShowToast) {
    globalShowToast(toast);
    return;
  }
  pendingToasts.push(toast);
}

export function notifyError(
  title: string,
  body?: string,
  options?: Omit<ToastInput, "variant" | "title" | "body">
) {
  notifyToast({
    variant: "error",
    title,
    body,
    ...options,
  });
}

// ─── Context ──────────────────────────────────────────────────────────────────

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}

// ─── Provider ─────────────────────────────────────────────────────────────────

const MAX_VISIBLE = 3;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const dismiss = useCallback((id: string) => {
    clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const show = useCallback(
    (incoming: Omit<Toast, "id">) => {
      const id = Math.random().toString(36).slice(2, 9);
      const toast: Toast = { ...incoming, id };

      setToasts((prev) => {
        // Queue beyond MAX_VISIBLE: keep the oldest visible, add to the end
        const next = [...prev, toast];
        return next;
      });

      // Auto-dismiss: errors never; others after duration (default 4s)
      const duration =
        incoming.duration !== undefined
          ? incoming.duration
          : incoming.variant === "error"
          ? 0
          : 4000;

      if (duration > 0) {
        const t = setTimeout(() => dismiss(id), duration);
        timers.current.set(id, t);
      }
    },
    [dismiss]
  );

  // Clean up timers on unmount
  useEffect(() => {
    return () => {
      timers.current.forEach(clearTimeout);
    };
  }, []);

  useEffect(() => {
    globalShowToast = show;

    if (pendingToasts.length > 0) {
      const queued = pendingToasts.splice(0, pendingToasts.length);
      queued.forEach((toast) => show(toast));
    }

    return () => {
      if (globalShowToast === show) {
        globalShowToast = null;
      }
    };
  }, [show]);

  // Only the first MAX_VISIBLE toasts are rendered (oldest first)
  const visible = toasts.slice(-MAX_VISIBLE);

  return (
    <ToastContext.Provider value={{ show, dismiss }}>
      {children}

      {/* aria-live region so screen readers announce toasts */}
      <div
        aria-live="polite"
        aria-atomic="false"
        className="toast-region"
        aria-label="Notifications"
      >
        {visible.map((toast) => (
          <ToastItem key={toast.id} toast={toast} onDismiss={dismiss} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

// ─── ToastItem ────────────────────────────────────────────────────────────────

const ICONS: Record<ToastVariant, React.ReactNode> = {
  success: <Check size={14} />,
  error:   <X size={14} />,
  warning: <AlertTriangle size={14} />,
  info:    <Info size={14} />,
};

function ToastItem({
  toast,
  onDismiss,
}: {
  toast: Toast;
  onDismiss: (id: string) => void;
}) {
  return (
    <div
      className={`toast toast--${toast.variant}`}
      role="alert"
      aria-live="assertive"
    >
      <span className="toast__icon" aria-hidden="true">
        {ICONS[toast.variant]}
      </span>

      <div className="toast__content">
        <p className="toast__title">{toast.title}</p>
        {toast.body && <p className="toast__body">{toast.body}</p>}
        {toast.action && (
          <button
            type="button"
            className="toast__action"
            onClick={() => {
              toast.action!.onClick();
              onDismiss(toast.id);
            }}
          >
            {toast.action.label}
          </button>
        )}
      </div>

      <button
        type="button"
        className="toast__close"
        onClick={() => onDismiss(toast.id)}
        aria-label="Dismiss notification"
      >
        <X size={12} />
      </button>
    </div>
  );
}
