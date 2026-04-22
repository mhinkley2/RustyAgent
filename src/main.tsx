import React, { Component, ErrorInfo, ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installFrontendLogging } from "./lib/logging";

installFrontendLogging();

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  componentDidCatch(error: Error, info: ErrorInfo) { console.error("React render error:", error, info); }
  render() {
    if (this.state.error) {
      const err = this.state.error as Error;
      return (
        <div style={{ padding: "2rem", fontFamily: "monospace", color: "#f85149", background: "#0d1117", minHeight: "100vh" }}>
          <h2 style={{ color: "#e6edf3" }}>App failed to render</h2>
          <pre style={{ whiteSpace: "pre-wrap", wordBreak: "break-all" }}>{err?.message}{"\n"}{err?.stack}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
