import { X } from "lucide-react";
import type { FileTab } from "../../hooks/useFileTabs";
import type { DiagnosticSeverity } from "../../hooks/useFileDiagnostics";

// ---------------------------------------------------------------------------
// EditorTabs
// ---------------------------------------------------------------------------

interface EditorTabsProps {
  tabs: FileTab[];
  activeTabPath: string | null;
  onSelect: (path: string) => void;
  onClose: (path: string) => void;
  diagnostics?: Map<string, DiagnosticSeverity>;
}

export function EditorTabs({ tabs, activeTabPath, onSelect, onClose, diagnostics }: EditorTabsProps) {
  if (tabs.length === 0) return null;

  return (
    <div className="editor-tabs" role="tablist">
      {tabs.map(tab => {
        const sev = diagnostics?.get(tab.path.toLowerCase()) ?? "none";
        const diagCls = sev === "error" ? " editor-tab--error" : sev === "warning" ? " editor-tab--warning" : "";
        return (
        <div
          key={tab.path}
          className={`editor-tab${tab.path === activeTabPath ? " editor-tab--active" : ""}${diagCls}`}
          role="tab"
          aria-selected={tab.path === activeTabPath}
          tabIndex={0}
          title={tab.path}
          onClick={() => onSelect(tab.path)}
          onKeyDown={e => {
            if (e.key === "Enter" || e.key === " ") onSelect(tab.path);
          }}
        >
          <span className="editor-tab__name">
            {tab.isDirty && <span className="editor-tab__dot" aria-label="unsaved changes" />}
            {tab.name}
          </span>
          <button
            className="editor-tab__close"
            title="Close tab"
            aria-label={`Close ${tab.name}`}
            onClick={e => {
              e.stopPropagation();
              onClose(tab.path);
            }}
          >
            <X size={11} />
          </button>
        </div>
        );
      })}
    </div>
  );
}
