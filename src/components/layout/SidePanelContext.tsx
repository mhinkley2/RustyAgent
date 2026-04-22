import { createContext, useContext, useState, useCallback, type ReactNode } from "react";

interface SidePanelContextValue {
  /** Whether the side panel is currently open. */
  open: boolean;
  setOpen: (open: boolean) => void;
  /** Active panel mode. */
  mode: "chat" | "activity";
  setMode: (mode: "chat" | "activity") => void;
  /** Current width in px (0 when closed). */
  width: number;
  setWidth: (width: number) => void;
  /** Content rendered inside the side panel — set by each page on mount. */
  panelContent: ReactNode;
  setPanelContent: (content: ReactNode) => void;
}

const SidePanelContext = createContext<SidePanelContextValue | null>(null);

const LS_KEY_OPEN = "rustyagent.sidepanel.open";
const LS_KEY_WIDTH = "rustyagent.sidepanel.width";
const LS_KEY_MODE = "rustyagent.sidepanel.mode";
const DEFAULT_WIDTH = 240;

function readLsBoolean(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    return v === "true";
  } catch {
    return fallback;
  }
}

function readLsNumber(key: string, fallback: number): number {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    const n = parseInt(v, 10);
    return isNaN(n) ? fallback : n;
  } catch {
    return fallback;
  }
}

export function SidePanelProvider({ children }: { children: ReactNode }) {
  const [open, setOpenRaw] = useState(() => readLsBoolean(LS_KEY_OPEN, false));
  const [width, setWidthRaw] = useState(() => readLsNumber(LS_KEY_WIDTH, DEFAULT_WIDTH));
  const [mode, setModeRaw] = useState<"chat" | "activity">(() => {
    try {
      const v = localStorage.getItem(LS_KEY_MODE);
      return v === "activity" ? "activity" : "chat";
    } catch {
      return "chat";
    }
  });
  const [panelContent, setPanelContent] = useState<ReactNode>(null);

  const setOpen = useCallback((v: boolean) => {
    setOpenRaw(v);
    try { localStorage.setItem(LS_KEY_OPEN, String(v)); } catch { /* ignore */ }
  }, []);

  const setWidth = useCallback((v: number) => {
    setWidthRaw(v);
    try { localStorage.setItem(LS_KEY_WIDTH, String(v)); } catch { /* ignore */ }
  }, []);

  const setMode = useCallback((v: "chat" | "activity") => {
    setModeRaw(v);
    try { localStorage.setItem(LS_KEY_MODE, v); } catch { /* ignore */ }
  }, []);

  return (
    <SidePanelContext.Provider value={{ open, setOpen, mode, setMode, width, setWidth, panelContent, setPanelContent }}>
      {children}
    </SidePanelContext.Provider>
  );
}

export function useSidePanel(): SidePanelContextValue {
  const ctx = useContext(SidePanelContext);
  if (!ctx) throw new Error("useSidePanel must be used within SidePanelProvider");
  return ctx;
}
