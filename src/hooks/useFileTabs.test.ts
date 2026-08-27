import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { tauriMock } from "../test/tauriMock";

let useFileTabs: typeof import("./useFileTabs").useFileTabs;

beforeEach(async () => {
  useFileTabs = (await import("./useFileTabs")).useFileTabs;
});

/** A filesystem the editor can read and write. */
function fs(initial: Record<string, string> = {}) {
  const files = { ...initial };
  tauriMock.handleAll({
    read_file_text: (args) => {
      const path = String(args.path);
      if (!(path in files)) throw new Error(`No such file: ${path}`);
      return files[path];
    },
    write_file_text: (args) => {
      files[String(args.path)] = String(args.content);
      return undefined;
    },
  });
  return files;
}

function render(workspaceId: string | null = "ws-1") {
  return renderHook(({ id }) => useFileTabs(id), {
    initialProps: { id: workspaceId as string | null },
  });
}

describe("useFileTabs — opening", () => {
  it("loads content, derives the name and language, and activates the tab", async () => {
    fs({ "/ws/src/main.rs": "fn main() {}" });
    const { result } = render();

    await act(async () => {
      await result.current.openFile("/ws/src/main.rs");
    });

    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0].name).toBe("main.rs");
    expect(result.current.tabs[0].language).toBe("rust");
    expect(result.current.tabs[0].content).toBe("fn main() {}");
    expect(result.current.tabs[0].isDirty).toBe(false);
    expect(result.current.activeTabPath).toBe("/ws/src/main.rs");
    expect(result.current.activeTab?.path).toBe("/ws/src/main.rs");
  });

  it("falls back to plaintext for an unknown extension", async () => {
    fs({ "/ws/notes.xyz": "text" });
    const { result } = render();

    await act(async () => {
      await result.current.openFile("/ws/notes.xyz");
    });

    expect(result.current.tabs[0].language).toBe("plaintext");
  });

  it("handles Windows-style paths when deriving the display name", async () => {
    fs({ "C:\\ws\\src\\App.tsx": "export default null;" });
    const { result } = render();

    await act(async () => {
      await result.current.openFile("C:\\ws\\src\\App.tsx");
    });

    expect(result.current.tabs[0].name).toBe("App.tsx");
    expect(result.current.tabs[0].language).toBe("typescript");
  });

  it("re-activates an already-open tab without re-reading the file", async () => {
    fs({ "/ws/a.ts": "a", "/ws/b.ts": "b" });
    const { result } = render();

    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });
    await act(async () => {
      await result.current.openFile("/ws/b.ts");
    });
    const readsBefore = tauriMock.callCount("read_file_text");

    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });

    expect(result.current.tabs).toHaveLength(2);
    expect(result.current.activeTabPath).toBe("/ws/a.ts");
    expect(tauriMock.callCount("read_file_text")).toBe(readsBefore);
  });

  it("records the error and opens no tab when the read fails", async () => {
    fs({});
    const { result } = render();

    await act(async () => {
      await result.current.openFile("/ws/missing.ts");
    });

    expect(result.current.tabs).toHaveLength(0);
    expect(result.current.error).toContain("No such file");
  });
});

describe("useFileTabs — dirty tracking and saving", () => {
  async function openOne() {
    fs({ "/ws/a.ts": "original" });
    const view = render();
    await act(async () => {
      await view.result.current.openFile("/ws/a.ts");
    });
    return view;
  }

  it("marks a tab dirty only when the content differs from disk", async () => {
    const { result } = await openOne();

    act(() => result.current.updateContent("/ws/a.ts", "changed"));

    expect(result.current.tabs[0].isDirty).toBe(true);
    expect(result.current.tabs[0].content).toBe("changed");
    expect(result.current.tabs[0].savedContent).toBe("original");
  });

  it("clears the dirty flag when the content is edited back to what was saved", async () => {
    const { result } = await openOne();

    act(() => result.current.updateContent("/ws/a.ts", "changed"));
    act(() => result.current.updateContent("/ws/a.ts", "original"));

    expect(result.current.tabs[0].isDirty).toBe(false);
  });

  it("leaves other tabs untouched when one is edited", async () => {
    fs({ "/ws/a.ts": "a", "/ws/b.ts": "b" });
    const { result } = render();
    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });
    await act(async () => {
      await result.current.openFile("/ws/b.ts");
    });

    act(() => result.current.updateContent("/ws/a.ts", "edited"));

    expect(result.current.tabs.find((t) => t.path === "/ws/b.ts")?.isDirty).toBe(false);
  });

  it("writes to disk, clears the dirty flag, and promotes the content", async () => {
    const files = fs({ "/ws/a.ts": "original" });
    const { result } = render();
    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });
    act(() => result.current.updateContent("/ws/a.ts", "changed"));

    await act(async () => {
      await result.current.saveTab("/ws/a.ts");
    });

    expect(files["/ws/a.ts"]).toBe("changed");
    expect(result.current.tabs[0].isDirty).toBe(false);
    expect(result.current.tabs[0].savedContent).toBe("changed");
    expect(result.current.tabs[0].saving).toBe(false);
  });

  it("keeps the tab dirty and records the error when the write fails", async () => {
    const { result } = await openOne();
    act(() => result.current.updateContent("/ws/a.ts", "changed"));
    tauriMock.handle("write_file_text", () => {
      throw new Error("disk is full");
    });

    await act(async () => {
      await result.current.saveTab("/ws/a.ts");
    });

    expect(result.current.tabs[0].isDirty).toBe(true);
    expect(result.current.tabs[0].saving).toBe(false);
    expect(result.current.error).toContain("disk is full");
  });

  it("saving an unknown path is a no-op", async () => {
    const { result } = await openOne();

    await act(async () => {
      await result.current.saveTab("/ws/not-open.ts");
    });

    expect(tauriMock.called("write_file_text")).toBe(false);
  });
});

describe("useFileTabs — closing", () => {
  async function openThree() {
    fs({ "/ws/a.ts": "a", "/ws/b.ts": "b", "/ws/c.ts": "c" });
    const view = render();
    for (const path of ["/ws/a.ts", "/ws/b.ts", "/ws/c.ts"]) {
      await act(async () => {
        await view.result.current.openFile(path);
      });
    }
    return view;
  }

  it("activates the tab to the left when the active tab is closed", async () => {
    const { result } = await openThree();
    expect(result.current.activeTabPath).toBe("/ws/c.ts");

    act(() => result.current.closeTab("/ws/c.ts"));

    expect(result.current.tabs.map((t) => t.path)).toEqual(["/ws/a.ts", "/ws/b.ts"]);
    expect(result.current.activeTabPath).toBe("/ws/b.ts");
  });

  it("activates the first tab when the leftmost tab is closed", async () => {
    const { result } = await openThree();
    act(() => result.current.setActiveTab("/ws/a.ts"));

    act(() => result.current.closeTab("/ws/a.ts"));

    expect(result.current.activeTabPath).toBe("/ws/b.ts");
  });

  it("leaves the active tab alone when a different tab is closed", async () => {
    const { result } = await openThree();

    act(() => result.current.closeTab("/ws/a.ts"));

    expect(result.current.activeTabPath).toBe("/ws/c.ts");
  });

  it("clears the active path when the last tab is closed", async () => {
    fs({ "/ws/a.ts": "a" });
    const { result } = render();
    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });

    act(() => result.current.closeTab("/ws/a.ts"));

    expect(result.current.tabs).toEqual([]);
    expect(result.current.activeTabPath).toBeNull();
    expect(result.current.activeTab).toBeNull();
  });
});

describe("useFileTabs — persistence", () => {
  it("persists the open paths and the active path under the workspace id", async () => {
    fs({ "/ws/a.ts": "a", "/ws/b.ts": "b" });
    const { result } = render("ws-1");

    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });
    await act(async () => {
      await result.current.openFile("/ws/b.ts");
    });

    await waitFor(() => {
      expect(JSON.parse(localStorage.getItem("editor-tabs:ws-1") ?? "[]")).toEqual([
        "/ws/a.ts",
        "/ws/b.ts",
      ]);
    });
    expect(localStorage.getItem("editor-active-tab:ws-1")).toBe("/ws/b.ts");
  });

  it("removes the active-tab key once no tab is active", async () => {
    fs({ "/ws/a.ts": "a" });
    const { result } = render("ws-1");
    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });
    await waitFor(() =>
      expect(localStorage.getItem("editor-active-tab:ws-1")).toBe("/ws/a.ts"),
    );

    act(() => result.current.closeTab("/ws/a.ts"));

    await waitFor(() =>
      expect(localStorage.getItem("editor-active-tab:ws-1")).toBeNull(),
    );
  });

  it("neither reads nor writes storage when there is no workspace", async () => {
    fs({ "/ws/a.ts": "a" });
    const { result } = render(null);

    await act(async () => {
      await result.current.openFile("/ws/a.ts");
    });

    expect(localStorage.length).toBe(0);
    expect(result.current.tabs).toHaveLength(1);
  });

  it("restores the saved tabs and active tab for a workspace", async () => {
    localStorage.setItem("editor-tabs:ws-1", JSON.stringify(["/ws/a.ts", "/ws/b.ts"]));
    localStorage.setItem("editor-active-tab:ws-1", "/ws/b.ts");
    fs({ "/ws/a.ts": "a", "/ws/b.ts": "b" });

    const { result } = render("ws-1");

    await waitFor(() =>
      expect(result.current.tabs.map((t) => t.path)).toEqual(["/ws/a.ts", "/ws/b.ts"]),
    );
    expect(result.current.activeTabPath).toBe("/ws/b.ts");
  });

  it("skips saved files that no longer read successfully", async () => {
    localStorage.setItem(
      "editor-tabs:ws-1",
      JSON.stringify(["/ws/gone.ts", "/ws/a.ts"]),
    );
    fs({ "/ws/a.ts": "a" });

    const { result } = render("ws-1");

    await waitFor(() => expect(result.current.tabs).toHaveLength(1));
    expect(result.current.tabs[0].path).toBe("/ws/a.ts");
  });

  it("falls back to the first tab when the saved active path is gone", async () => {
    localStorage.setItem("editor-tabs:ws-1", JSON.stringify(["/ws/a.ts"]));
    localStorage.setItem("editor-active-tab:ws-1", "/ws/deleted.ts");
    fs({ "/ws/a.ts": "a" });

    const { result } = render("ws-1");

    await waitFor(() => expect(result.current.tabs).toHaveLength(1));
    expect(result.current.activeTabPath).toBe("/ws/a.ts");
  });

  it("yields an empty tab set when the stored JSON is corrupt", async () => {
    localStorage.setItem("editor-tabs:ws-1", "{not json");
    fs({});

    const { result } = render("ws-1");

    await waitFor(() => expect(result.current.tabs).toEqual([]));
    expect(tauriMock.called("read_file_text")).toBe(false);
  });

  it("saves the outgoing workspace's tabs then restores the incoming one's", async () => {
    localStorage.setItem("editor-tabs:ws-2", JSON.stringify(["/ws2/x.ts"]));
    fs({ "/ws1/a.ts": "a", "/ws2/x.ts": "x" });

    const { result, rerender } = render("ws-1");
    await act(async () => {
      await result.current.openFile("/ws1/a.ts");
    });
    await waitFor(() =>
      expect(localStorage.getItem("editor-tabs:ws-1")).toContain("/ws1/a.ts"),
    );

    rerender({ id: "ws-2" });

    // The incoming workspace's tabs are restored...
    await waitFor(() =>
      expect(result.current.tabs.map((t) => t.path)).toEqual(["/ws2/x.ts"]),
    );
    // ...and the outgoing workspace's are still on record for when we go back.
    expect(JSON.parse(localStorage.getItem("editor-tabs:ws-1") ?? "[]")).toEqual([
      "/ws1/a.ts",
    ]);
  });

  it("restores the original workspace's tabs when switching back", async () => {
    fs({ "/ws1/a.ts": "a", "/ws2/x.ts": "x" });
    const { result, rerender } = render("ws-1");
    await act(async () => {
      await result.current.openFile("/ws1/a.ts");
    });
    await waitFor(() =>
      expect(localStorage.getItem("editor-tabs:ws-1")).toContain("/ws1/a.ts"),
    );

    rerender({ id: "ws-2" });
    await waitFor(() => expect(result.current.tabs).toEqual([]));
    await act(async () => {
      await result.current.openFile("/ws2/x.ts");
    });

    rerender({ id: "ws-1" });

    await waitFor(() =>
      expect(result.current.tabs.map((t) => t.path)).toEqual(["/ws1/a.ts"]),
    );
  });
});
