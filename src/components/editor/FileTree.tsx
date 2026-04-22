import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronRight, ChevronDown, Folder, FolderOpen, File,
  FileCode, FileText, FileJson,
} from "lucide-react";
import { ContextMenu, type ContextMenuTarget } from "./ContextMenu";
import type { DiagnosticSeverity } from "../../hooks/useFileDiagnostics";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}

interface TreeNode extends FileEntry {
  children?: TreeNode[];
  loaded: boolean;
  expanded: boolean;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const CODE_EXTS = new Set(["ts","tsx","js","jsx","rs","py","go","rb","java","c","cpp","h","hpp","cs","php","swift","kt","lua","sh","bash","dart","zig","ex","exs","clj","fs","ml"]);
const JSON_EXTS = new Set(["json","jsonc","toml","yaml","yml","xml","csv","env"]);

function FileIcon({ name, isDir, expanded }: { name: string; isDir: boolean; expanded: boolean }) {
  if (isDir) {
    return expanded
      ? <FolderOpen size={14} className="ft__icon ft__icon--dir" />
      : <Folder size={14} className="ft__icon ft__icon--dir" />;
  }
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (CODE_EXTS.has(ext)) return <FileCode size={14} className="ft__icon ft__icon--code" />;
  if (JSON_EXTS.has(ext)) return <FileJson size={14} className="ft__icon ft__icon--json" />;
  if (ext === "md" || ext === "txt" || ext === "log") return <FileText size={14} className="ft__icon ft__icon--text" />;
  return <File size={14} className="ft__icon" />;
}

/** Returns the worst diagnostic severity for a given folder path (checks all descendants). */
function getFolderSeverity(
  folderPath: string,
  diagnostics: Map<string, DiagnosticSeverity>,
): DiagnosticSeverity {
  let result: DiagnosticSeverity = "none";
  const lower = folderPath.toLowerCase();
  const prefix = lower.endsWith("/") || lower.endsWith("\\")
    ? lower
    : lower + "\\";
  const fwdPrefix = lower.split("\\").join("/") + "/";
  for (const [filePath, sev] of diagnostics.entries()) {
    if (filePath.startsWith(prefix) || filePath.startsWith(fwdPrefix)) {
      if (sev === "error") return "error";
      if (sev === "warning") result = "warning";
    }
  }
  return result;
}

function diagClass(sev: DiagnosticSeverity): string {
  if (sev === "error") return " tree-node--error";
  if (sev === "warning") return " tree-node--warning";
  return "";
}

// ---------------------------------------------------------------------------
// Rename input (inline, auto-focused)
// ---------------------------------------------------------------------------

function RenameInput({
  initialValue,
  onConfirm,
  onCancel,
}: {
  initialValue: string;
  onConfirm: (name: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    // Select filename without extension
    const dotIdx = initialValue.lastIndexOf(".");
    inputRef.current?.setSelectionRange(0, dotIdx > 0 ? dotIdx : initialValue.length);
  }, [initialValue]);

  return (
    <input
      ref={inputRef}
      className="ft__rename-input"
      value={value}
      onChange={e => setValue(e.target.value)}
      onKeyDown={e => {
        if (e.key === "Enter") { e.preventDefault(); if (value.trim()) onConfirm(value.trim()); }
        if (e.key === "Escape") { e.preventDefault(); onCancel(); }
        e.stopPropagation();
      }}
      onBlur={() => onCancel()}
      onClick={e => e.stopPropagation()}
    />
  );
}

// ---------------------------------------------------------------------------
// TreeItem
// ---------------------------------------------------------------------------

interface TreeItemProps {
  node: TreeNode;
  depth: number;
  onSelect: (node: TreeNode) => void;
  onToggle: (path: string) => void;
  onContextMenuOpen: (e: React.MouseEvent, node: TreeNode) => void;
  activePath: string | null;
  diagnostics: Map<string, DiagnosticSeverity>;
  editingPath: string | null;
  onRenameConfirm: (oldPath: string, newName: string) => void;
  onRenameCancel: () => void;
  pendingNew: { parentPath: string; type: "file" | "dir"; } | null;
  onPendingNewConfirm: (name: string) => void;
  onPendingNewCancel: () => void;
}

function TreeItem({
  node, depth, onSelect, onToggle, onContextMenuOpen,
  activePath, diagnostics,
  editingPath, onRenameConfirm, onRenameCancel,
  pendingNew, onPendingNewConfirm, onPendingNewCancel,
}: TreeItemProps) {
  const isActive = activePath === node.path;
  const isEditing = editingPath === node.path;
  const sev = node.is_dir
    ? getFolderSeverity(node.path, diagnostics)
    : (diagnostics.get(node.path.toLowerCase()) ?? "none");

  const itemDepth = 8 + depth * 14;
  const childDepth = 8 + (depth + 1) * 14;

  return (
    <>
      <div
        className={`ft__item${isActive ? " ft__item--active" : ""}${diagClass(sev)}`}
        style={{ paddingLeft: itemDepth }}
        onClick={() => {
          if (isEditing) return;
          if (node.is_dir) onToggle(node.path);
          else onSelect(node);
        }}
        onContextMenu={e => { e.preventDefault(); onContextMenuOpen(e, node); }}
        role="treeitem"
        aria-selected={isActive}
        aria-expanded={node.is_dir ? node.expanded : undefined}
        tabIndex={0}
        onKeyDown={e => {
          if (e.key === "Enter" || e.key === " ") {
            if (node.is_dir) onToggle(node.path);
            else onSelect(node);
          }
        }}
      >
        <span className="ft__chevron">
          {node.is_dir
            ? (node.expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />)
            : null}
        </span>
        <FileIcon name={node.name} isDir={node.is_dir} expanded={node.expanded} />
        {isEditing ? (
          <RenameInput
            initialValue={node.name}
            onConfirm={newName => onRenameConfirm(node.path, newName)}
            onCancel={onRenameCancel}
          />
        ) : (
          <span className={`ft__name${diagClass(sev)}`}>{node.name}</span>
        )}
      </div>

      {/* Pending new file/folder input inside expanded directory */}
      {node.is_dir && node.expanded && pendingNew?.parentPath === node.path && (
        <div className="ft__new-item" style={{ paddingLeft: childDepth }}>
          <span className="ft__chevron" />
          {pendingNew.type === "dir"
            ? <Folder size={14} className="ft__icon ft__icon--dir" />
            : <File size={14} className="ft__icon" />}
          <RenameInput
            initialValue={pendingNew.type === "file" ? "untitled" : "new-folder"}
            onConfirm={onPendingNewConfirm}
            onCancel={onPendingNewCancel}
          />
        </div>
      )}

      {/* Children */}
      {node.is_dir && node.expanded && node.children && node.children.map(child => (
        <TreeItem
          key={child.path}
          node={child}
          depth={depth + 1}
          onSelect={onSelect}
          onToggle={onToggle}
          onContextMenuOpen={onContextMenuOpen}
          activePath={activePath}
          diagnostics={diagnostics}
          editingPath={editingPath}
          onRenameConfirm={onRenameConfirm}
          onRenameCancel={onRenameCancel}
          pendingNew={pendingNew}
          onPendingNewConfirm={onPendingNewConfirm}
          onPendingNewCancel={onPendingNewCancel}
        />
      ))}
    </>
  );
}

// ---------------------------------------------------------------------------
// FileTree
// ---------------------------------------------------------------------------

export interface FileTreeProps {
  rootPath: string;
  onFileSelect: (path: string) => void;
  onCloseTab?: (path: string) => void;
  activePath: string | null;
  diagnostics?: Map<string, DiagnosticSeverity>;
}

export function FileTree({
  rootPath,
  onFileSelect,
  onCloseTab,
  activePath,
  diagnostics = new Map(),
}: FileTreeProps) {
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [rootLoaded, setRootLoaded] = useState(false);
  const [loadingPath, setLoadingPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Context menu
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; target: ContextMenuTarget } | null>(null);
  // Inline rename
  const [editingPath, setEditingPath] = useState<string | null>(null);
  // Pending new item (file/folder)
  const [pendingNew, setPendingNew] = useState<{ parentPath: string; type: "file" | "dir" } | null>(null);

  // Reset tree when workspace changes
  useEffect(() => {
    setNodes([]);
    setRootLoaded(false);
    setLoadingPath(null);
    setError(null);
    setContextMenu(null);
    setEditingPath(null);
    setPendingNew(null);
  }, [rootPath]);

  // Load root when not yet loaded
  useEffect(() => {
    if (rootLoaded || !rootPath) return;
    let cancelled = false;
    (async () => {
      setLoadingPath(rootPath);
      try {
        const entries = await invoke<FileEntry[]>("list_directory", { path: rootPath });
        if (!cancelled) {
          setNodes(entries.map(e => ({ ...e, loaded: false, expanded: false })));
          setRootLoaded(true);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setLoadingPath(null);
      }
    })();
    return () => { cancelled = true; };
  }, [rootLoaded, rootPath]);

  // ---------------------------------------------------------------------------
  // Tree helpers
  // ---------------------------------------------------------------------------

  const refresh = useCallback(() => {
    setNodes([]);
    setRootLoaded(false);
    setLoadingPath(null);
  }, []);

  const updateNode = useCallback((path: string, updater: (n: TreeNode) => TreeNode) => {
    const walk = (items: TreeNode[]): TreeNode[] =>
      items.map(n => {
        if (n.path === path) return updater(n);
        if (n.children) return { ...n, children: walk(n.children) };
        return n;
      });
    setNodes(prev => walk(prev));
  }, []);

  const findNode = useCallback((path: string, items = nodes): TreeNode | null => {
    for (const n of items) {
      if (n.path === path) return n;
      if (n.children) {
        const found = findNode(path, n.children);
        if (found) return found;
      }
    }
    return null;
  }, [nodes]);

  const expandDir = useCallback(async (path: string) => {
    const node = findNode(path);
    if (!node || !node.is_dir) return;
    if (node.expanded) return; // already open

    if (!node.loaded) {
      setLoadingPath(path);
      try {
        const entries = await invoke<FileEntry[]>("list_directory", { path });
        const children: TreeNode[] = entries.map(e => ({ ...e, loaded: false, expanded: false }));
        updateNode(path, n => ({ ...n, children, loaded: true, expanded: true }));
      } finally {
        setLoadingPath(null);
      }
    } else {
      updateNode(path, n => ({ ...n, expanded: true }));
    }
  }, [findNode, updateNode]);

  const toggleDir = useCallback(async (path: string) => {
    const node = findNode(path);
    if (!node || !node.is_dir) return;

    if (!node.loaded && !node.expanded) {
      setLoadingPath(path);
      try {
        const entries = await invoke<FileEntry[]>("list_directory", { path });
        const children: TreeNode[] = entries.map(e => ({ ...e, loaded: false, expanded: false }));
        updateNode(path, n => ({ ...n, children, loaded: true, expanded: true }));
      } finally {
        setLoadingPath(null);
      }
    } else {
      updateNode(path, n => ({ ...n, expanded: !n.expanded }));
    }
  }, [findNode, updateNode]);

  // ---------------------------------------------------------------------------
  // Context menu actions
  // ---------------------------------------------------------------------------

  const handleContextMenuOpen = useCallback((e: React.MouseEvent, node: TreeNode) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ x: e.clientX, y: e.clientY, target: { path: node.path, name: node.name, is_dir: node.is_dir } });
  }, []);

  const handleRename = useCallback(() => {
    if (!contextMenu) return;
    setEditingPath(contextMenu.target.path);
  }, [contextMenu]);

  const handleRenameConfirm = useCallback(async (oldPath: string, newName: string) => {
    setEditingPath(null);
    try {
      await invoke<string>("rename_path", { oldPath, newName });
      refresh();
    } catch (e) {
      setError(`Rename failed: ${e}`);
    }
  }, [refresh]);

  const handleDuplicate = useCallback(async () => {
    if (!contextMenu) return;
    try {
      const newPath = await invoke<string>("duplicate_file", { path: contextMenu.target.path });
      refresh();
      onFileSelect(newPath);
    } catch (e) {
      setError(`Duplicate failed: ${e}`);
    }
  }, [contextMenu, refresh, onFileSelect]);

  const handleDelete = useCallback(async () => {
    if (!contextMenu) return;
    const { path, name, is_dir } = contextMenu.target;
    const msg = is_dir
      ? `Delete folder "${name}" and all its contents?`
      : `Delete "${name}"?`;
    if (!window.confirm(msg)) return;
    try {
      await invoke("delete_path", { path });
      if (onCloseTab) onCloseTab(path);
      refresh();
    } catch (e) {
      setError(`Delete failed: ${e}`);
    }
  }, [contextMenu, onCloseTab, refresh]);

  const handleNewFile = useCallback(async () => {
    if (!contextMenu) return;
    const { path, is_dir } = contextMenu.target;
    const parentPath = is_dir ? path : (path.substring(0, Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"))) || path);
    await expandDir(parentPath);
    setPendingNew({ parentPath, type: "file" });
  }, [contextMenu, expandDir]);

  const handleNewFolder = useCallback(async () => {
    if (!contextMenu) return;
    const { path, is_dir } = contextMenu.target;
    const parentPath = is_dir ? path : (path.substring(0, Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"))) || path);
    await expandDir(parentPath);
    setPendingNew({ parentPath, type: "dir" });
  }, [contextMenu, expandDir]);

  const handlePendingNewConfirm = useCallback(async (name: string) => {
    if (!pendingNew) return;
    const sep = pendingNew.parentPath.includes("\\") ? "\\" : "/";
    const newPath = pendingNew.parentPath + sep + name;
    try {
      if (pendingNew.type === "file") {
        const created = await invoke<string>("create_empty_file", { path: newPath });
        setPendingNew(null);
        refresh();
        onFileSelect(created);
      } else {
        await invoke<string>("create_dir_fs", { path: newPath });
        setPendingNew(null);
        refresh();
      }
    } catch (e) {
      setError(`Create failed: ${e}`);
      setPendingNew(null);
    }
  }, [pendingNew, refresh, onFileSelect]);

  const handleCopyPath = useCallback(() => {
    if (!contextMenu) return;
    navigator.clipboard.writeText(contextMenu.target.path).catch(() => {});
  }, [contextMenu]);

  const handleCopyRelativePath = useCallback(() => {
    if (!contextMenu) return;
    const rel = contextMenu.target.path.replace(rootPath, "").replace(/^[/\\]/, "");
    navigator.clipboard.writeText(rel).catch(() => {});
  }, [contextMenu, rootPath]);

  const handleRevealInExplorer = useCallback(() => {
    if (!contextMenu) return;
    invoke("reveal_in_explorer", { path: contextMenu.target.path }).catch(e => setError(String(e)));
  }, [contextMenu]);

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  if (!rootLoaded && !error) {
    return (
      <div className="ft__skeleton" aria-label="Loading workspace files">
        {[0, 1, 2, 3, 4].map(i => (
          <div key={i} className="ft__skeleton-row" />
        ))}
      </div>
    );
  }

  if (error) {
    return <div className="ft__error">{error}</div>;
  }

  const sharedItemProps = {
    onSelect: (node: TreeNode) => onFileSelect(node.path),
    onToggle: toggleDir,
    onContextMenuOpen: handleContextMenuOpen,
    activePath,
    diagnostics,
    editingPath,
    onRenameConfirm: handleRenameConfirm,
    onRenameCancel: () => setEditingPath(null),
    pendingNew,
    onPendingNewConfirm: handlePendingNewConfirm,
    onPendingNewCancel: () => setPendingNew(null),
  };

  return (
    <>
      <div className="ft" role="tree" aria-label="File tree">
        {nodes.map(node => (
          <TreeItem key={node.path} node={node} depth={0} {...sharedItemProps} />
        ))}
        {loadingPath && <div className="ft__loading">Loading…</div>}
      </div>

      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          target={contextMenu.target}
          workspaceRoot={rootPath}
          onClose={() => setContextMenu(null)}
          onRename={handleRename}
          onDuplicate={handleDuplicate}
          onDelete={handleDelete}
          onNewFile={handleNewFile}
          onNewFolder={handleNewFolder}
          onCopyPath={handleCopyPath}
          onCopyRelativePath={handleCopyRelativePath}
          onRevealInExplorer={handleRevealInExplorer}
        />
      )}
    </>
  );
}
