import { useCallback, useEffect, useRef, useState } from "react";
import type { StoryStatus } from "../../types/board";
import { KANBAN_COLUMNS } from "../../types/board";

const STORAGE_KEY = "rustyagent.board.collapsedColumns";

/**
 * The collapsed set as it survives a remount, read back defensively.
 *
 * Anything unrecognised is dropped rather than trusted: the value is whatever
 * a previous build of the app wrote, and a status that no longer exists would
 * otherwise sit in the set collapsing a column nobody can find. Storage can
 * also throw outright — a private window, or site data blocked — so the whole
 * read is guarded and an unreadable store simply means nothing is collapsed.
 */
export function parseCollapsed(raw: string | null): Set<StoryStatus> {
  if (!raw) return new Set();
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    const known = new Set<string>(KANBAN_COLUMNS.map(c => c.status));
    return new Set(parsed.filter((v): v is StoryStatus => typeof v === "string" && known.has(v)));
  } catch {
    return new Set();
  }
}

function read(): Set<StoryStatus> {
  try {
    return parseCollapsed(localStorage.getItem(STORAGE_KEY));
  } catch {
    return new Set();
  }
}

function write(collapsed: Set<StoryStatus>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...collapsed]));
  } catch {
    /* ignore — a collapsed column is not worth failing a render over */
  }
}

/**
 * Which Kanban columns are collapsed, remembered across remounts.
 *
 * The board unmounts whenever the side panel opens over it or the user leaves
 * the page, so collapse state held in plain component state would come back
 * expanded every time — which reads as the control not working rather than as
 * state being lost.
 */
export function useCollapsedColumns() {
  const [collapsed, setCollapsed] = useState<Set<StoryStatus>>(read);

  // The updater stays pure. Writing to storage inside it happens to be
  // idempotent under StrictMode's double invocation today, and stops being so
  // the moment two toggles are batched — a reducer is the wrong place to
  // notice that.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      // Nothing to persist on mount: this is the value that was just read back.
      first.current = false;
      return;
    }
    write(collapsed);
  }, [collapsed]);

  const toggle = useCallback((status: StoryStatus) => {
    setCollapsed(prev => {
      const next = new Set(prev);
      if (next.has(status)) next.delete(status);
      else next.add(status);
      return next;
    });
  }, []);

  return { collapsed, toggle };
}
