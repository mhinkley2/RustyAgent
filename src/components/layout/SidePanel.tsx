import { useRef, useCallback, type ReactNode } from "react";
import { useSidePanel } from "./SidePanelContext";

const MIN_WIDTH = 160;
const MAX_WIDTH = 480;
const SNAP_CLOSE_THRESHOLD = 20;

interface SidePanelProps {
  children?: ReactNode;
}

export default function SidePanel({ children }: SidePanelProps) {
  const { open, setOpen, width, setWidth, panelContent } = useSidePanel();
  const dragging = useRef(false);
  const panelRef = useRef<HTMLDivElement>(null);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      dragging.current = true;

      // Disable CSS transition during drag for performance.
      if (panelRef.current) {
        panelRef.current.style.transition = "none";
      }

      const startX = e.clientX;
      const startWidth = open ? width : 0;

      function onMouseMove(ev: MouseEvent) {
        const delta = ev.clientX - startX;
        const newWidth = startWidth + delta;

        if (newWidth <= SNAP_CLOSE_THRESHOLD) {
          // Snap closed.
          setOpen(false);
          return;
        }

        const clamped = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, newWidth));
        setWidth(clamped);
        if (!open) setOpen(true);
      }

      function onMouseUp(ev: MouseEvent) {
        window.removeEventListener("mousemove", onMouseMove);
        window.removeEventListener("mouseup", onMouseUp);
        dragging.current = false;

        // Re-enable transition.
        if (panelRef.current) {
          panelRef.current.style.transition = "";
        }

        // Snap to nearest named width on mouse-up.
        const delta = ev.clientX - startX;
        const newWidth = startWidth + delta;
        if (newWidth <= SNAP_CLOSE_THRESHOLD) {
          setOpen(false);
          return;
        }
        const snaps = [MIN_WIDTH, 240, 360, MAX_WIDTH];
        const clamped = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, newWidth));
        const nearest = snaps.reduce((a, b) =>
          Math.abs(a - clamped) <= Math.abs(b - clamped) ? a : b
        );
        setWidth(nearest);
        setOpen(true);
      }

      window.addEventListener("mousemove", onMouseMove);
      window.addEventListener("mouseup", onMouseUp);
    },
    [open, width, setOpen, setWidth]
  );

  const currentWidth = open ? width : 0;

  return (
    <div
      ref={panelRef}
      className={["side-panel", open ? "side-panel--open" : ""].join(" ")}
      style={{ width: currentWidth }}
      aria-hidden={!open}
    >
      <div className="side-panel__content">
        {panelContent ?? children ?? null}
      </div>

      {/* Drag handle */}
      <div
        className="side-panel__handle"
        onMouseDown={handleMouseDown}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize side panel"
      />
    </div>
  );
}
