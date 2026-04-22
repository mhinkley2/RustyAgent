import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useEffect, useRef } from "react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface FileTab {
  /** Absolute path to the file. */
  path: string;
  /** Display name (last path component). */
  name: string;
  /** Current content in the editor (may differ from disk). */
  content: string;
  /** Content as it was when loaded from disk. */
  savedContent: string;
  /** True when content !== savedContent. */
  isDirty: boolean;
  /** Language identifier for Monaco. */
  language: string;
  /** True while saving. */
  saving: boolean;
}

// ---------------------------------------------------------------------------
// Language detection from file extension
// ---------------------------------------------------------------------------

const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript", tsx: "typescript",
  js: "javascript", jsx: "javascript",
  rs: "rust",
  py: "python",
  json: "json", jsonc: "json",
  toml: "toml",
  yaml: "yaml", yml: "yaml",
  md: "markdown",
  html: "html", htm: "html",
  css: "css",
  scss: "scss", sass: "scss",
  sh: "shell", bash: "shell",
  sql: "sql",
  xml: "xml",
  txt: "plaintext",
  go: "go",
  rb: "ruby",
  java: "java",
  c: "c", h: "c",
  cpp: "cpp", cc: "cpp", cxx: "cpp", hpp: "cpp",
  cs: "csharp",
  php: "php",
  swift: "swift",
  kt: "kotlin",
  r: "r",
  lua: "lua",
};

function detectLanguage(filename: string): string {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  return EXT_TO_LANG[ext] ?? "plaintext";
}

// ---------------------------------------------------------------------------
// Workspace tab persistence helpers
// ---------------------------------------------------------------------------

function tabsStorageKey(workspaceId: string): string {
  return `editor-tabs:${workspaceId}`;
}

function activeTabStorageKey(workspaceId: string): string {
  return `editor-active-tab:${workspaceId}`;
}

function saveTabsToStorage(workspaceId: string | null | undefined, paths: string[], activePath: string | null) {
  if (!workspaceId) return;
  try {
    localStorage.setItem(tabsStorageKey(workspaceId), JSON.stringify(paths));
    if (activePath) {
      localStorage.setItem(activeTabStorageKey(workspaceId), activePath);
    } else {
      localStorage.removeItem(activeTabStorageKey(workspaceId));
    }
  } catch {
    // ignore quota errors
  }
}

function loadSavedPaths(workspaceId: string | null | undefined): { paths: string[]; activePath: string | null } {
  if (!workspaceId) return { paths: [], activePath: null };
  try {
    const raw = localStorage.getItem(tabsStorageKey(workspaceId));
    const paths: string[] = raw ? JSON.parse(raw) : [];
    const activePath = localStorage.getItem(activeTabStorageKey(workspaceId));
    return { paths, activePath };
  } catch {
    return { paths: [], activePath: null };
  }
}

// ---------------------------------------------------------------------------
// useFileTabs
// ---------------------------------------------------------------------------

interface UseFileTabsReturn {
  tabs: FileTab[];
  activeTabPath: string | null;
  activeTab: FileTab | null;
  openFile: (path: string) => Promise<void>;
  closeTab: (path: string) => void;
  setActiveTab: (path: string) => void;
  updateContent: (path: string, content: string) => void;
  saveTab: (path: string) => Promise<void>;
  error: string | null;
}

export function useFileTabs(workspaceId?: string | null): UseFileTabsReturn {
  const [tabs, setTabs] = useState<FileTab[]>([]);
  const [activeTabPath, setActiveTabPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Track previous workspace so we can save before clearing.
  const prevWorkspaceRef = useRef<string | null | undefined>(workspaceId);

  // When workspaceId changes: save current state, clear tabs, restore saved paths.
  useEffect(() => {
    const prevId = prevWorkspaceRef.current;
    prevWorkspaceRef.current = workspaceId;

    if (prevId === workspaceId) return;

    // Save tabs for the outgoing workspace.
    setTabs(prev => {
      setActiveTabPath(active => {
        saveTabsToStorage(prevId, prev.map(t => t.path), active);
        return null;
      });
      return [];
    });
  }, [workspaceId]);

  // After tabs are cleared (workspace just changed), restore saved paths.
  const restoredRef = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    if (restoredRef.current === workspaceId) return;
    if (tabs.length > 0) return; // not cleared yet
    restoredRef.current = workspaceId;

    if (!workspaceId) return;
    const { paths, activePath } = loadSavedPaths(workspaceId);
    if (paths.length === 0) return;

    // Reopen files sequentially; failures are silently skipped.
    (async () => {
      const opened: FileTab[] = [];
      for (const path of paths) {
        try {
          const content = await invoke<string>("read_file_text", { path });
          const name = path.split(/[/\\]/).pop() ?? path;
          opened.push({
            path, name, content, savedContent: content,
            isDirty: false, language: detectLanguage(name), saving: false,
          });
        } catch {
          // file no longer exists or inaccessible — skip it
        }
      }
      if (opened.length > 0) {
        setTabs(opened);
        const restoreActive = activePath && opened.some(t => t.path === activePath)
          ? activePath
          : opened[0].path;
        setActiveTabPath(restoreActive);
      }
    })();
  }, [workspaceId, tabs.length]);

  // Persist tab paths whenever tabs or activeTabPath change.
  useEffect(() => {
    saveTabsToStorage(workspaceId, tabs.map(t => t.path), activeTabPath);
  }, [workspaceId, tabs, activeTabPath]);

  const openFile = useCallback(async (path: string) => {
    // If already open, just switch to it
    const existing = tabs.find(t => t.path === path);
    if (existing) {
      setActiveTabPath(path);
      return;
    }

    try {
      const content = await invoke<string>("read_file_text", { path });
      const name = path.split(/[/\\]/).pop() ?? path;
      const tab: FileTab = {
        path,
        name,
        content,
        savedContent: content,
        isDirty: false,
        language: detectLanguage(name),
        saving: false,
      };
      setTabs(prev => [...prev, tab]);
      setActiveTabPath(path);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [tabs]);

  const closeTab = useCallback((path: string) => {
    setTabs(prev => {
      const idx = prev.findIndex(t => t.path === path);
      const next = prev.filter(t => t.path !== path);
      setActiveTabPath(active => {
        if (active !== path) return active;
        if (next.length === 0) return null;
        // Activate the tab to the left, or the first
        const newIdx = Math.max(0, idx - 1);
        return next[newIdx]?.path ?? null;
      });
      return next;
    });
  }, []);

  const updateContent = useCallback((path: string, content: string) => {
    setTabs(prev => prev.map(t =>
      t.path === path
        ? { ...t, content, isDirty: content !== t.savedContent }
        : t
    ));
  }, []);

  const saveTab = useCallback(async (path: string) => {
    const tab = tabs.find(t => t.path === path);
    if (!tab) return;
    setTabs(prev => prev.map(t => t.path === path ? { ...t, saving: true } : t));
    try {
      await invoke("write_file_text", { path, content: tab.content });
      setTabs(prev => prev.map(t =>
        t.path === path
          ? { ...t, saving: false, isDirty: false, savedContent: t.content }
          : t
      ));
      setError(null);
    } catch (e) {
      setTabs(prev => prev.map(t => t.path === path ? { ...t, saving: false } : t));
      setError(String(e));
    }
  }, [tabs]);

  const activeTab = tabs.find(t => t.path === activeTabPath) ?? null;

  return {
    tabs,
    activeTabPath,
    activeTab,
    openFile,
    closeTab,
    setActiveTab: setActiveTabPath,
    updateContent,
    saveTab,
    error,
  };
}
