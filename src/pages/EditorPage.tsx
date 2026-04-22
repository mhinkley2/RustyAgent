import { useCallback, useEffect, useMemo } from "react";
import MonacoEditor from "@monaco-editor/react";
import { marked } from "marked";
import { FolderOpen, FolderSearch, History, Save, Clock } from "lucide-react";

import { useWorkspaceContext } from "../context/WorkspaceContext";
import { useFileTabs } from "../hooks/useFileTabs";
import { useFileDiagnostics } from "../hooks/useFileDiagnostics";
import { FileTree } from "../components/editor/FileTree";
import { EditorTabs } from "../components/editor/EditorTabs";
import "../styles/editor.css";

// ---------------------------------------------------------------------------
// EditorPage
// ---------------------------------------------------------------------------

export default function EditorPage() {
  const {
    activeWorkspace,
    recentWorkspaces,
    loading: workspaceLoading,
    pickAndOpenWorkspace,
    reopenWorkspace,
    removeWorkspace,
  } = useWorkspaceContext();

  const {
    tabs,
    activeTabPath,
    activeTab,
    openFile,
    closeTab,
    setActiveTab,
    updateContent,
    saveTab,
    error: fileError,
  } = useFileTabs(activeWorkspace?.id);

  const diagnostics = useFileDiagnostics(activeWorkspace?.path ?? null);

  // Ctrl+S — save active tab
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s" && activeTabPath) {
        e.preventDefault();
        saveTab(activeTabPath);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeTabPath, saveTab]);

  const handleSave = useCallback(() => {
    if (activeTabPath) saveTab(activeTabPath);
  }, [activeTabPath, saveTab]);

  // ── No workspace open yet ──────────────────────────────────────────────────
  if (!workspaceLoading && !activeWorkspace) {
    return (
      <div className="editor-page editor-page--empty">
        <div className="editor-welcome">
          <FolderSearch size={40} className="editor-welcome__icon" />
          <h2 className="editor-welcome__title">Open a workspace</h2>
          <p className="editor-welcome__desc">
            Select a project folder to browse and edit files.
          </p>
          <button
            className="btn btn--primary"
            onClick={pickAndOpenWorkspace}
          >
            <FolderOpen size={14} />
            Open folder…
          </button>

          {recentWorkspaces.length > 0 && (
            <div className="editor-welcome__recent">
              <h3 className="editor-welcome__recent-title">
                <History size={13} /> Recent workspaces
              </h3>
              <ul className="editor-welcome__recent-list">
                {recentWorkspaces.map(ws => (
                  <li key={ws.id} className="editor-welcome__recent-item">
                    <button
                      className="editor-welcome__recent-btn"
                      onClick={() => reopenWorkspace(ws)}
                      title={ws.path}
                    >
                      <span className="editor-welcome__recent-name">{ws.name}</span>
                      <span className="editor-welcome__recent-path">{ws.path}</span>
                    </button>
                    <button
                      className="editor-welcome__recent-remove"
                      aria-label="Remove from recent"
                      onClick={() => removeWorkspace(ws.id)}
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </div>
    );
  }

  // ── Editor layout ──────────────────────────────────────────────────────────
  return (
    <div className="editor-page">
      {/* ── Sidebar ─────────────────────────────────────────────────── */}
      <div className="editor-sidebar">
        <div className="editor-sidebar__header">
          <span className="editor-sidebar__workspace-name" title={activeWorkspace?.path}>
            {activeWorkspace?.name ?? "…"}
          </span>
          <button
            className="btn btn--ghost btn--xs"
            title="Open a different folder"
            aria-label="Open folder"
            onClick={pickAndOpenWorkspace}
          >
            <FolderOpen size={13} />
          </button>
        </div>
        <div className="editor-sidebar__tree">
          {activeWorkspace && (
            <FileTree
              rootPath={activeWorkspace.path}
              onFileSelect={openFile}
              onCloseTab={closeTab}
              activePath={activeTabPath}
              diagnostics={diagnostics}
            />
          )}
        </div>
      </div>

      {/* ── Main area ───────────────────────────────────────────────── */}
      <div className="editor-main">
        {/* Tab bar */}
        <div className="editor-main__tabbar">
          <EditorTabs
            tabs={tabs}
            activeTabPath={activeTabPath}
            onSelect={setActiveTab}
            onClose={closeTab}
            diagnostics={diagnostics}
          />
          {activeTab && (
            <div className="editor-main__actions">
              {activeTab.saving && (
                <span className="editor-main__saving">
                  <Clock size={12} /> Saving…
                </span>
              )}
              <button
                className={`btn btn--ghost btn--xs${activeTab.isDirty ? " editor-main__save-btn--dirty" : ""}`}
                onClick={handleSave}
                disabled={!activeTab.isDirty || activeTab.saving}
                title="Save (Ctrl+S)"
                aria-label="Save file"
              >
                <Save size={13} />
                Save
              </button>
            </div>
          )}
        </div>

        {/* Error banner */}
        {fileError && (
          <div className="editor-main__error" role="alert">
            {fileError}
          </div>
        )}

        {/* Monaco editor or Markdown preview */}
        {activeTab ? (
          activeTab.language === "markdown" ? (
            <MarkdownPreview content={activeTab.content} />
          ) : (
            <div className="editor-main__monaco">
              <MonacoEditor
                key={activeTab.path}
                path={`file:///${activeTab.path.replace(/^\\\\\?\\/, '').replace(/\\/g, '/').replace(/^\/+/, '')}`}
                language={activeTab.language}
                value={activeTab.content}
                theme="vs-dark"
                options={{
                  fontSize: 13,
                  fontFamily: "var(--font-mono), 'Cascadia Code', Menlo, monospace",
                  minimap: { enabled: tabs.length > 0 },
                  scrollBeyondLastLine: false,
                  wordWrap: "on",
                  lineNumbers: "on",
                  renderWhitespace: "selection",
                  tabSize: 2,
                  automaticLayout: true,
                }}
                onChange={value => {
                  if (value !== undefined) updateContent(activeTab.path, value);
                }}
                onMount={(_editor, monaco) => {
                  // Disable semantic validation — Monaco can't resolve imports
                  // from the user's file system, so these are always false positives.
                  monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions({
                    noSemanticValidation: true,
                    noSuggestionDiagnostics: true,
                  });
                  monaco.languages.typescript.javascriptDefaults.setDiagnosticsOptions({
                    noSemanticValidation: true,
                    noSuggestionDiagnostics: true,
                  });
                  monaco.editor.defineTheme("rustyagent-dark", {
                    base: "vs-dark",
                    inherit: true,
                    rules: [],
                    colors: { "editor.background": "#0f1117" },
                  });
                  monaco.editor.setTheme("rustyagent-dark");
                }}
              />
            </div>
          )
        ) : (
          <div className="editor-main__placeholder">
            <p>Select a file from the tree to open it.</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// MarkdownPreview
// ---------------------------------------------------------------------------

function MarkdownPreview({ content }: { content: string }) {
  const html = useMemo(() => {
    return marked.parse(content, { async: false }) as string;
  }, [content]);

  return (
    <div
      className="editor-md-preview"
      // marked output is HTML — XSS safe for local files the user opened themselves.
      // eslint-disable-next-line react/no-danger
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
