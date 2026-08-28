import { act } from "@testing-library/react";
import { vi } from "vitest";

// ---------------------------------------------------------------------------
// A shared in-memory stand-in for the Tauri IPC bridge.
//
// The registries below are module-level rather than per-factory on purpose.
// Several components (ChatPage most of all) register a `listen("run-event")`
// handler once per *module load* and keep it in a module-level singleton, so a
// fresh listener Set per test would silently orphan that handler and make the
// suite order-dependent. Keeping one registry and clearing it in `reset()`
// — paired with `vi.resetModules()` in setup — gives each test a clean, and
// crucially an *independent*, slate.
// ---------------------------------------------------------------------------

type InvokeArgs = Record<string, unknown>;
type Handler = (args: InvokeArgs) => unknown | Promise<unknown>;
type Listener = (event: { payload: unknown }) => void;

const handlers = new Map<string, Handler>();
const listeners = new Map<string, Set<Listener>>();
const recorded: { command: string; args: InvokeArgs }[] = [];

export const invokeMock = vi.fn(async (command: string, args?: InvokeArgs) => {
  recorded.push({ command, args: args ?? {} });
  const handler = handlers.get(command);
  if (!handler) {
    throw new Error(`Unhandled invoke command: ${command}`);
  }
  return handler(args ?? {});
});

export const listenMock = vi.fn(async (eventName: string, callback: Listener) => {
  let set = listeners.get(eventName);
  if (!set) {
    set = new Set();
    listeners.set(eventName, set);
  }
  set.add(callback);
  return () => {
    set!.delete(callback);
  };
});

export const tauriMock = {
  /** Register (or replace) the response for one invoke command. */
  handle(command: string, fn: Handler) {
    handlers.set(command, fn);
    return this;
  },

  /** Register several commands at once. */
  handleAll(map: Record<string, Handler>) {
    for (const [command, fn] of Object.entries(map)) this.handle(command, fn);
    return this;
  },

  /** Dispatch a backend event to every subscriber, wrapped in `act`. */
  async emit(eventName: string, payload: unknown) {
    await act(async () => {
      for (const callback of [...(listeners.get(eventName) ?? [])]) {
        callback({ payload });
      }
    });
  },

  /** Arguments of every call to `command`, in order. */
  calls(command: string): InvokeArgs[] {
    return recorded.filter((c) => c.command === command).map((c) => c.args);
  },

  callCount(command: string) {
    return this.calls(command).length;
  },

  /** Whether `command` was ever invoked. */
  called(command: string) {
    return this.callCount(command) > 0;
  },

  /** How many live subscribers an event name has — used to assert cleanup. */
  listenerCount(eventName: string) {
    return listeners.get(eventName)?.size ?? 0;
  },

  reset() {
    handlers.clear();
    listeners.clear();
    recorded.length = 0;
    invokeMock.mockClear();
    listenMock.mockClear();
  },
};
