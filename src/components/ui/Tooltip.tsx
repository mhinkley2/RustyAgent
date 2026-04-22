import { useEffect, useRef, useState } from "react";

// ─── Tooltip ────────────────────────────────────────────────────────────────

interface TooltipProps {
  /** The text shown in the tooltip. Keep it brief. */
  content: string;
  children: React.ReactElement;
  /** Delay before showing the tooltip. Defaults to 200ms. */
  delayMs?: number;
}

/**
 * Tooltip — appears above the child after a 200ms hover delay.
 * For icon-only buttons this wrapper provides the visible label.
 * Never put interactive content inside a tooltip — use Popover instead.
 *
 * Usage:
 *   <Tooltip content="Edit agent">
 *     <IconButton aria-label="Edit agent" icon={<Pencil />} />
 *   </Tooltip>
 */
export function Tooltip({ content, children, delayMs = 200 }: TooltipProps) {
  const [visible, setVisible] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  function show() {
    timerRef.current = setTimeout(() => setVisible(true), delayMs);
  }

  function hide() {
    if (timerRef.current) clearTimeout(timerRef.current);
    setVisible(false);
  }

  useEffect(() => () => { if (timerRef.current) clearTimeout(timerRef.current); }, []);

  // Clone child adding the hover handlers + aria-describedby
  const tooltipId = `tt-${content.slice(0, 12).replace(/\s/g, "-")}`;

  const child = children as React.ReactElement<React.HTMLAttributes<Element>>;

  const enhanced = {
    ...child,
    props: {
      ...child.props,
      onMouseEnter: (e: React.MouseEvent<Element>) => { show(); child.props.onMouseEnter?.(e); },
      onMouseLeave: (e: React.MouseEvent<Element>) => { hide(); child.props.onMouseLeave?.(e); },
      onFocus:      (e: React.FocusEvent<Element>) => { show(); child.props.onFocus?.(e); },
      onBlur:       (e: React.FocusEvent<Element>) => { hide(); child.props.onBlur?.(e); },
      "aria-describedby": visible ? tooltipId : child.props["aria-describedby"],
    },
  };

  return (
    <span className="tooltip-wrap" style={{ position: "relative", display: "inline-flex" }}>
      {enhanced as React.ReactElement}
      {visible && (
        <span
          id={tooltipId}
          role="tooltip"
          className="tooltip"
        >
          {content}
        </span>
      )}
    </span>
  );
}

// ─── Popover ─────────────────────────────────────────────────────────────────

interface PopoverProps {
  /** The trigger element. Clicking it toggles the popover. */
  trigger: React.ReactElement;
  /** Content rendered inside the popover panel. */
  children: React.ReactNode;
  /** Panel placement. Defaults to "bottom-end". */
  placement?: "bottom-start" | "bottom-end" | "top-start" | "top-end";
}

/**
 * Popover — click-triggered floating panel.
 * Focused when opened. Closes on outside click or Escape.
 * Used for overflow menus, filter panels, cron previews.
 *
 * Never put interactive content in a Tooltip — use Popover instead.
 */
export function Popover({ trigger, children, placement = "bottom-end" }: PopoverProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    }
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open]);

  // Focus panel when opened
  useEffect(() => {
    if (open) requestAnimationFrame(() => panelRef.current?.focus());
  }, [open]);

  const triggerEl = trigger as React.ReactElement<React.HTMLAttributes<Element>>;
  const enhancedTrigger = {
    ...triggerEl,
    props: {
      ...triggerEl.props,
      onClick: (e: React.MouseEvent<Element>) => {
        setOpen((v) => !v);
        triggerEl.props.onClick?.(e);
      },
      "aria-expanded": open,
      "aria-haspopup": "true" as const,
    },
  };

  const placementClass = `popover--${placement}`;

  return (
    <div ref={containerRef} style={{ position: "relative", display: "inline-flex" }}>
      {enhancedTrigger as React.ReactElement}
      {open && (
        <div
          ref={panelRef}
          className={`popover ${placementClass}`}
          role="dialog"
          tabIndex={-1}
        >
          {children}
        </div>
      )}
    </div>
  );
}
