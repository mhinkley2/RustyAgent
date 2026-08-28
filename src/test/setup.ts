import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { invokeMock, listenMock, tauriMock } from "./tauriMock";

// The Tauri bridge is mocked for every test file. Individual tests declare the
// commands they care about via `tauriMock.handle(...)`; anything unhandled
// throws, so a test can never silently pass against a command that does not
// exist.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) =>
    (invokeMock as unknown as (...a: unknown[]) => unknown)(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) =>
    (listenMock as unknown as (...a: unknown[]) => unknown)(...args),
}));

// Pure-logic test files opt into the `node` environment for speed, where
// there is no DOM to clean up.
const hasDom = typeof window !== "undefined";

beforeEach(() => {
  // Clear the IPC registries *before* resetting modules, so a component that
  // registers a module-level listener on first import lands in a clean set.
  tauriMock.reset();
  if (hasDom) window.localStorage.clear();
  vi.resetModules();
});

afterEach(() => {
  if (hasDom) {
    cleanup();
    window.localStorage.clear();
  }
  tauriMock.reset();
});
