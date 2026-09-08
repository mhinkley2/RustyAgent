import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";

import { DEFAULT_FILTERS, FilterBar, activeCount, type BoardFilters } from "./FilterBar";

/**
 * The bar as `BoardPage` drives it: state above, changes flowing back down.
 *
 * A `FilterBar` rendered with a fixed `filters` prop cannot show what the
 * debounce does, because the prop never catches up with the box.
 */
function Harness({ onChange }: { onChange?: (f: BoardFilters) => void }) {
  const [filters, setFilters] = useState<BoardFilters>(DEFAULT_FILTERS);
  return (
    <FilterBar
      filters={filters}
      onChange={next => {
        setFilters(next);
        onChange?.(next);
      }}
      availableLabels={["board", "mcp"]}
    />
  );
}

const searchBox = () => screen.getByRole("searchbox", { name: /search stories/i });

// A fake-timer test that fails leaves them installed, and every test after it
// in this file then hangs on a timer that never fires. Restoring here means one
// broken assertion stays one broken assertion.
afterEach(() => {
  vi.useRealTimers();
});

describe("activeCount", () => {
  it("counts a search as one filter, so it clears with the others", () => {
    expect(activeCount({ ...DEFAULT_FILTERS, search: "cap" })).toBe(1);
  });

  // Otherwise the "Clear 1 filter" button appears over a board nothing is
  // filtering, because a trailing space looked like a query.
  it("does not count whitespace as a search", () => {
    expect(activeCount({ ...DEFAULT_FILTERS, search: "   " })).toBe(0);
  });

  it("counts each selected status", () => {
    expect(activeCount({ ...DEFAULT_FILTERS, statuses: ["ready", "done"] })).toBe(2);
  });

  it("adds the dimensions together", () => {
    expect(
      activeCount({
        quick: "mine",
        priorities: ["high"],
        types: [],
        labels: ["board"],
        statuses: ["done"],
        search: "cap",
      }),
    ).toBe(5);
  });
});

describe("FilterBar search", () => {
  it("waits for typing to settle, then filters once for the whole word", async () => {
    vi.useFakeTimers();
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    // `fireEvent` rather than `userEvent` here: this test is about our own
    // timer, and userEvent runs a timer of its own between keystrokes that
    // deadlocks against faked ones. Every other test in this file uses
    // userEvent, where the real input path is what is being checked.
    for (const value of ["c", "ca", "cap"]) {
      fireEvent.change(searchBox(), { target: { value } });
    }

    // Mid-word the board has not been asked to re-filter at all — three
    // keystrokes are one query, not three passes over every story.
    expect(onChange).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0].search).toBe("cap");
  });

  it("shows every keystroke immediately, whatever the debounce is doing", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(searchBox(), "diff");
    expect(searchBox()).toHaveValue("diff");
  });

  it("clears from the box's own button", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    await user.type(searchBox(), "cap");
    await user.click(screen.getByRole("button", { name: /clear search/i }));

    expect(searchBox()).toHaveValue("");
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ search: "" }));
  });

  // Escape is the reflex, and the box is inside a toolbar with no other way
  // back to an unfiltered board without reaching for the mouse.
  it("clears on Escape", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(searchBox(), "cap");
    await user.keyboard("{Escape}");

    expect(searchBox()).toHaveValue("");
  });

  // The box holds its own draft between keystrokes, so clearing the state is
  // not enough on its own — this is the case that regresses.
  it("empties the box when the bar's clear-all is pressed", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(searchBox(), "cap");
    await user.click(await screen.findByRole("button", { name: /clear 1 filter/i }));

    expect(searchBox()).toHaveValue("");
    expect(screen.queryByRole("button", { name: /clear \d+ filter/i })).toBeNull();
  });
});

describe("FilterBar status", () => {
  it("offers every column as a status, and reports the one picked", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);

    await user.click(screen.getByRole("checkbox", { name: "In Progress" }));

    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ statuses: ["in_progress"] }),
    );
  });
});
