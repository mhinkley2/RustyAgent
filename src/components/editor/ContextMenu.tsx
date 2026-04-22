import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import {
  Pencil,
  Copy,
  Trash2,
  FilePlus,
  FolderPlus,
  Clipboard,
  ExternalLink,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ContextMenuTarget {
  path: string;
  name: string;
  is_dir: boolean;
}

export interface ContextMenuProps {
  x: number;
  y: number;
  target: ContextMenuTarget;
  workspaceRoot: string;
  onClose: () => void;
  onRename: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onNewFile: () => void;
  onNewFolder: () => void;
  onCopyPath: () => void;
  onCopyRelativePath: () => void;
  onRevealInExplorer: () => void;
}

// ---------------------------------------------------------------------------
// ContextMenu
// ---------------------------------------------------------------------------

interface MenuItemProps {
  icon: React.ReactNode;
  label: string;
  danger?: boolean;
  onClick: () => void;
}

function MenuItem({ icon, label, danger, onClick }: MenuItemProps) {
  return (
    <button
      className={`ctx-menu__item${danger ? " ctx-menu__item--danger" : ""}`}
      onClick={onClick}
      role="menuitem"
    >
      <span className="ctx-menu__item-icon" aria-hidden="true">{icon}</span>
      <span className="ctx-menu__item-label">{label}</span>
    </button>
  );
}

function Separator() {
  return <div className="ctx-menu__separator" role="separator" />;
}

export function ContextMenu({
  x,
  y,
  target,
  onClose,
  onRename,
  onDuplicate,
  onDelete,
  onNewFile,
  onNewFolder,
  onCopyPath,
  onCopyRelativePath,
  onRevealInExplorer,
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  // Clamp to viewport so menu never clips off-screen
  const MENU_W = 220;
  const MENU_H = target.is_dir ? 260 : 230;
  const left = Math.min(x, window.innerWidth - MENU_W - 8);
  const top = Math.min(y, window.innerHeight - MENU_H - 8);

  const wrap = (fn: () => void) => () => { fn(); onClose(); };

  return createPortal(
    <div
      ref={menuRef}
      className="ctx-menu"
      role="menu"
      aria-label="File context menu"
      style={{ position: "fixed", left, top }}
    >
      {/* New File / New Folder (always at top for dirs) */}
      {target.is_dir && (
        <>
          <MenuItem icon={<FilePlus size={13} />} label="New File" onClick={wrap(onNewFile)} />
          <MenuItem icon={<FolderPlus size={13} />} label="New Folder" onClick={wrap(onNewFolder)} />
          <Separator />
        </>
      )}

      {!target.is_dir && (
        <MenuItem icon={<FilePlus size={13} />} label="New File Here" onClick={wrap(onNewFile)} />
      )}

      <MenuItem icon={<Pencil size={13} />} label="Rename" onClick={wrap(onRename)} />
      {!target.is_dir && (
        <MenuItem icon={<Copy size={13} />} label="Duplicate" onClick={wrap(onDuplicate)} />
      )}
      <MenuItem icon={<Trash2 size={13} />} label="Delete" danger onClick={wrap(onDelete)} />

      <Separator />

      <MenuItem icon={<Clipboard size={13} />} label="Copy Path" onClick={wrap(onCopyPath)} />
      <MenuItem icon={<Clipboard size={13} />} label="Copy Relative Path" onClick={wrap(onCopyRelativePath)} />
      <MenuItem icon={<ExternalLink size={13} />} label="Reveal in Explorer" onClick={wrap(onRevealInExplorer)} />
    </div>,
    document.body,
  );
}
