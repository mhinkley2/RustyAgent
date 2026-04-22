import { useEffect, useRef, useState } from "react";
import { useMonaco } from "@monaco-editor/react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type DiagnosticSeverity = "error" | "warning" | "none";

// ---------------------------------------------------------------------------
// useFileDiagnostics
//
// Subscribes to Monaco's onDidChangeMarkers and maintains a persistent Map
// from absolute file path → worst diagnostic severity.
//
// Key behaviour: diagnostics are only CLEARED when Monaco explicitly signals
// zero markers on a model that still exists (i.e. the error was actually
// fixed). Switching tabs or closing a file does not clear entries — model
// disposal fires onDidChangeMarkers with an empty list, but getModel() returns
// null in that case, so we keep the cached severity.
//
// Debounced 150 ms to avoid thrash when many models update at once.
// ---------------------------------------------------------------------------

export function useFileDiagnostics(
  workspacePath: string | null,
): Map<string, DiagnosticSeverity> {
  const monaco = useMonaco();
  // Accumulated map persists across renders; entries only removed when fixed.
  const accumulated = useRef<Map<string, DiagnosticSeverity>>(new Map());
  const [diagnostics, setDiagnostics] = useState<Map<string, DiagnosticSeverity>>(new Map());

  useEffect(() => {
    if (!monaco || !workspacePath) {
      accumulated.current = new Map();
      setDiagnostics(new Map());
      return;
    }

    const monacoApi = monaco;

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    function normPath(uri: any): string {
      return ((uri.fsPath ?? uri.path) as string).toLowerCase().replace(/\//g, "\\");
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    function updateForUris(uris: readonly any[]) {
      let changed = false;

      for (const uri of uris) {
        const filePath = normPath(uri);
        const markers = monacoApi.editor.getModelMarkers({ resource: uri });
        const model = monacoApi.editor.getModel(uri);

        let hasError = false;
        let hasWarning = false;
        for (const m of markers) {
          if (m.severity === monacoApi.MarkerSeverity.Error) { hasError = true; break; }
          if (m.severity === monacoApi.MarkerSeverity.Warning) hasWarning = true;
        }
        const sev: DiagnosticSeverity = hasError ? "error" : hasWarning ? "warning" : "none";
        const prev = accumulated.current.get(filePath);

        if (sev === "none") {
          // Only remove the cached diagnostic if the model still exists with
          // zero markers — meaning the user actually fixed the error.
          // If model is null, it was disposed (tab switch/close) — keep the entry.
          if (model !== null && prev !== undefined) {
            accumulated.current.delete(filePath);
            changed = true;
          }
        } else if (prev !== sev) {
          accumulated.current.set(filePath, sev);
          changed = true;
        }
      }

      if (changed) {
        setDiagnostics(new Map(accumulated.current));
      }
    }

    // Seed from all currently-loaded models.
    updateForUris(monacoApi.editor.getModels().map((m) => m.uri));

    // Incremental updates — only re-check the URIs that actually changed.
    let timer: ReturnType<typeof setTimeout>;
    const disposable = monacoApi.editor.onDidChangeMarkers((uris) => {
      clearTimeout(timer);
      timer = setTimeout(() => updateForUris(uris), 150);
    });

    return () => {
      disposable.dispose();
      clearTimeout(timer);
    };
  }, [monaco, workspacePath]);

  return diagnostics;
}

