import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { KanbanView } from "./KanbanView";
import type { Story, StoryStatus } from "../../types/board";

/**
 * Which element, if any, each column handed to dnd-kit as its drop target.
 *
 * dnd-kit registers a droppable through a ref callback and leaves no mark in
 * the DOM, so a column that forgets to attach it looks identical to one that
 * did. This records the calls, which is the only way to assert the difference
 * without simulating a drag jsdom cannot measure.
 */
const droppableNodes = vi.hoisted(() => new Map<string, HTMLElement | null>());

vi.mock("@dnd-kit/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@dnd-kit/core")>();
  return {
    ...actual,
    useDroppable: (args: Parameters<typeof actual.useDroppable>[0]) => {
      const real = actual.useDroppable(args);
      return {
        ...real,
        setNodeRef: (el: HTMLElement | null) => {
          droppableNodes.set(String(args.id), el);
          real.setNodeRef(el);
        },
      };
    },
  };
});

function story(id: string, status: StoryStatus, title = `Story ${id}`): Story {
  return {
    id,
    key: `#${id}`,
    title,
    status,
    priority: "medium",
    type: "task",
    labels: [],
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    createdAt: new Date("2026-08-31T00:00:00Z"),
    updatedAt: new Date("2026-08-31T00:00:00Z"),
  };
}

const done = (n: number) =>
  Array.from({ length: n }, (_, i) => story(`d${i}`, "done", `Done ${i}`));

function view(stories: Story[], props: Partial<Parameters<typeof KanbanView>[0]> = {}) {
  return render(
    <KanbanView
      stories={stories}
      onSelect={() => {}}
      onMove={async () => {}}
      onReorder={async () => {}}
      {...props}
    />,
  );
}

/** The column element for a status, found by the attribute the view sets. */
function column(status: StoryStatus): HTMLElement {
  const el = document.querySelector(`.kb-col[data-status="${status}"]`);
  if (!el) throw new Error(`no ${status} column rendered`);
  return el as HTMLElement;
}

beforeEach(() => {
  localStorage.clear();
  droppableNodes.clear();
});

afterEach(() => {
  localStorage.clear();
});

describe("KanbanView Done column", () => {
  // Done only grows. Rendering all of it makes the column the tallest thing on
  // the board and then keeps going.
  it("caps the cards it draws and offers the rest", async () => {
    const user = userEvent.setup();
    view(done(14));

    expect(within(column("done")).getAllByText(/^Done \d+$/)).toHaveLength(10);
    // The count in the header is the whole column, not what fits.
    expect(within(column("done")).getByText("14")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show 4 more" }));
    expect(within(column("done")).getAllByText(/^Done \d+$/)).toHaveLength(14);

    await user.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(within(column("done")).getAllByText(/^Done \d+$/)).toHaveLength(10);
  });

  it("does not offer an expander for a column that fits", () => {
    view(done(4));

    expect(within(column("done")).getAllByText(/^Done \d+$/)).toHaveLength(4);
    expect(screen.queryByRole("button", { name: /show \d+ more/i })).toBeNull();
  });

  // Backlog is not capped: it is the column you are working out of.
  it("caps only Done", () => {
    const backlog = Array.from({ length: 14 }, (_, i) =>
      story(`b${i}`, "backlog", `Backlog ${i}`),
    );
    view(backlog);

    expect(within(column("backlog")).getAllByText(/^Backlog \d+$/)).toHaveLength(14);
  });
});

describe("KanbanView column collapse", () => {
  it("collapses a column to its header and count", async () => {
    const user = userEvent.setup();
    view([story("a", "review", "In review")]);

    expect(screen.getByText("In review")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /collapse review column/i }));

    expect(screen.queryByText("In review")).toBeNull();
    expect(within(column("review")).getByText("1")).toBeInTheDocument();
  });

  // The board unmounts whenever the side panel opens over it, so state held
  // only in the component would come back expanded every time — which reads as
  // the control not working rather than as state being lost.
  it("stays collapsed across a remount", async () => {
    const user = userEvent.setup();
    const stories = [story("a", "review", "In review")];

    const first = view(stories);
    await user.click(screen.getByRole("button", { name: /collapse review column/i }));
    first.unmount();

    view(stories);
    expect(screen.queryByText("In review")).toBeNull();
    expect(screen.getByRole("button", { name: /expand review column/i })).toBeInTheDocument();
  });

  // Collapsing Done to get it out of the way and then being unable to drop a
  // finished card into it is exactly the workflow the collapse is for.
  it("stays a drop target while collapsed", async () => {
    const user = userEvent.setup();
    view([story("a", "review", "In review")]);

    await user.click(screen.getByRole("button", { name: /collapse review column/i }));

    const node = droppableNodes.get("review");
    expect(node).toBeTruthy();
    expect(column("review").contains(node!)).toBe(true);
  });

  it("expands again", async () => {
    const user = userEvent.setup();
    view([story("a", "review", "In review")]);

    await user.click(screen.getByRole("button", { name: /collapse review column/i }));
    await user.click(screen.getByRole("button", { name: /expand review column/i }));

    expect(screen.getByText("In review")).toBeInTheDocument();
  });

  it("ignores a stored column this build no longer draws", () => {
    localStorage.setItem("rustyagent.board.collapsedColumns", '["archived"]');
    view([story("a", "review", "In review")]);

    expect(screen.getByText("In review")).toBeInTheDocument();
  });
});

describe("KanbanView visible statuses", () => {
  // The status filter and the column layout should say the same thing, rather
  // than leaving five columns of "Nothing here" because one status was asked
  // for.
  it("draws only the statuses the filter asked for", () => {
    view([story("a", "ready"), story("b", "done")], { visibleStatuses: ["ready"] });

    expect(document.querySelector('.kb-col[data-status="ready"]')).not.toBeNull();
    expect(document.querySelector('.kb-col[data-status="done"]')).toBeNull();
  });

  it("draws every column when no status is selected", () => {
    view([story("a", "ready")], { visibleStatuses: [] });

    expect(document.querySelectorAll(".kb-col")).toHaveLength(6);
  });
});

describe("KanbanView localStorage failures", () => {
  // A private window, or site data blocked. A collapsed column is not worth
  // failing a render over.
  it("renders when storage cannot be read", () => {
    const getItem = vi
      .spyOn(Storage.prototype, "getItem")
      .mockImplementation(() => {
        throw new Error("blocked");
      });
    try {
      view([story("a", "ready", "Queued")]);
      expect(screen.getByText("Queued")).toBeInTheDocument();
    } finally {
      getItem.mockRestore();
    }
  });
});
