import { createContext, useContext, useEffect, useState, useCallback, ReactNode } from "react";

interface ModalContextValue {
  /** True when any modal/panel is open. */
  isOpen: boolean;
  /** Open a named panel/modal. */
  open: (id: string) => void;
  /** Close a specific named panel/modal, or close all if omitted. */
  close: (id?: string) => void;
  /** Whether a specific panel/modal is open. */
  isModalOpen: (id: string) => boolean;
}

const ModalContext = createContext<ModalContextValue | null>(null);

export function ModalProvider({ children }: { children: ReactNode }) {
  const [openModals, setOpenModals] = useState<Set<string>>(new Set());

  const open = useCallback((id: string) => {
    setOpenModals((prev) => new Set(prev).add(id));
  }, []);

  const close = useCallback((id?: string) => {
    if (id) {
      setOpenModals((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } else {
      setOpenModals(new Set());
    }
  }, []);

  const isModalOpen = useCallback(
    (id: string) => openModals.has(id),
    [openModals]
  );

  // Global Escape key → close all open modals/panels.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && openModals.size > 0) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [openModals, close]);

  return (
    <ModalContext.Provider value={{ isOpen: openModals.size > 0, open, close, isModalOpen }}>
      {children}
    </ModalContext.Provider>
  );
}

export function useModal() {
  const ctx = useContext(ModalContext);
  if (!ctx) throw new Error("useModal must be used inside <ModalProvider>");
  return ctx;
}
