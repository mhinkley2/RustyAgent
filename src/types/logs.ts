export type LogLevel = "debug" | "info" | "warn" | "error";

export interface FrontendLogEntry {
  id: string;
  timestamp: string;
  level: LogLevel;
  scope: string;
  message: string;
  details?: string;
  origin: "console" | "runtime" | "manual";
}

export interface BackendLogPayload {
  path: string;
  content: string;
}