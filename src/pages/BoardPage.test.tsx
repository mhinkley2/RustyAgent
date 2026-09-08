import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { tauriMock } from "../test/tauriMock";
import BoardPage from "./BoardPage";

/**
 * The board's wiring, end to end, through the real hooks and the real IPC mock.
 *
 * This file was written alongside the fix in PR #24 and then deleted before
 * merge — not because it failed, but because mounting `BoardPage` transitively
 * imports `RunPanel`, `ListView` and `StoryForm`, and coverage was
 * import-driven, so pulling four untested files into the denominator dropped
 * every global threshold below its floor. A test that caught the bug it was
 * written for could not be merged; deleting it made the build green. That is
 * the incentive `coverage.include` exists to remove, and this file is the thing
 * it was blocking.
 *
 * What it pins is the defect PR #24 fixed, which lived entirely in the wiring:
 * both routes to a blocked run went through "the first *non-dismissed*
 * request", which is `null` in exactly the case you would reach for it.
 */

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function rawStory(id: string, title: string, status = "in_progress") {
  return {
    id,
    title,
    description: null,
    story_type: "task",
    status,
    priority: "medium",
    assigned_agent_id: null,
    assigned_agent_name: null,
    requires_approval: false,
    track_history: true,
    labels: [],
    latest_run: null,
    sort_order: 0,
    created_at: "2026-08-31T00:00:00Z",
    updated_at: "2026-08-31T00:00:00Z",
  };
}

/** A pending question. `task_story_id` is the card it should mark. */
function rawHuman(id: string, taskStoryId: string | null, question = "Which one?") {
  return {
    id,
    story_id: `human-${id}`,
    task_story_id: taskStoryId,
    story_title: "Needs input",
    run_id: `run-${id}`,
    question,
    status: "pending",
    created_at: "2026-08-31T00:00:00Z",
  };
}

function rawApproval(id: string, storyId: string | null) {
  return {
    id,
    run_id: `run-${id}`,
    story_id: storyId,
    story_title: "Wants a tool",
    tool_name: "write_file",
    tool_input: '{"path":"x"}',
    status: "pending",
    created_at: "2026-08-31T00:00:00Z",
  };
}

interface BoardFixture {
  stories?: ReturnType<typeof rawStory>[];
  humans?: ReturnType<typeof rawHuman>[];
  approvals?: ReturnType<typeof rawApproval>[];
}

function renderBoard({ stories = [], humans = [], approvals = [] }: BoardFixture = {}) {
  tauriMock.handleAll({
    get_stories: () => stories,
    get_profiles: () => [],
    get_pending_human_requests: () => humans,
    get_pending_approvals: () => approvals,
    respond_to_human_request: () => null,
    decide_approval: () => null,
  });

  // BoardPage calls `useNavigate`, so it needs a router around it. Memory
  // history keeps the test off the URL bar.
  return render(
    <MemoryRouter>
      <BoardPage />
    </MemoryRouter>,
  );
}

const marker = (title: string) =>
  within(screen.getByLabelText(new RegExp(`: ${title}$`))).getByRole("button", {
    name: /waiting on you/i,
  });

const inputDialog = () => screen.queryByRole("dialog", { name: /asking for input/i });
const approvalDialog = () => screen.queryByRole("dialog", { name: /approve tool execution/i });

/**
 * A banner's own action button.
 *
 * Scoped to the banner rather than looked up globally: "Review" is also the
 * name of a Kanban column, whose collapse control is a button too.
 */
function bannerButton(kind: "input" | "approval") {
  const banner = document.querySelector(
    kind === "input" ? ".hitl-banner--input" : ".hitl-banner--approval",
  );
  if (!banner) throw new Error(`no ${kind} banner rendered`);
  return within(banner as HTMLElement).getByRole("button");
}

// ---------------------------------------------------------------------------

describe("BoardPage attention markers", () => {
  it("marks the card whose run is waiting, not the synthetic human story", async () => {
    renderBoard({
      stories: [rawStory("s1", "Blocked work"), rawStory("s2", "Fine")],
      humans: [rawHuman("h1", "s1")],
    });

    expect(await screen.findByText("Blocked work")).toBeInTheDocument();
    expect(marker("Blocked work")).toBeInTheDocument();

    // The card that is not waiting carries no marker.
    expect(
      within(screen.getByLabelText(/: Fine$/)).queryByRole("button", {
        name: /waiting on you/i,
      }),
    ).toBeNull();
  });

  it("opens that card's question, not whichever request happens to be first", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "First"), rawStory("s2", "Second")],
      humans: [rawHuman("h1", "s1", "Question for first"), rawHuman("h2", "s2", "Question for second")],
    });

    await screen.findByText("Second");
    await user.click(marker("Second"));

    expect(inputDialog()).toBeInTheDocument();
    expect(screen.getByText("Question for second")).toBeInTheDocument();
  });

  // With an unrelated question also outstanding. The two dialogs never stack,
  // so a focused approval has to *suppress* the input dialog — otherwise
  // pressing an approval marker while any question is pending opens the other
  // one, which is the same class of wrongness the fix removed.
  it("opens the approval gate even while a question is outstanding elsewhere", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "Wants to write"), rawStory("s2", "Also asking")],
      humans: [rawHuman("h1", "s2", "Unrelated question")],
      approvals: [rawApproval("a1", "s1")],
    });

    await screen.findByText("Wants to write");
    await user.click(marker("Wants to write"));

    expect(approvalDialog()).toBeInTheDocument();
    expect(inputDialog()).toBeNull();
  });

  // The two dialogs never stack, so a story with both pending must resolve to
  // one of them — and the marker must open the one the board will actually
  // show. Opening the approval and being handed the input dialog would be its
  // own small lie.
  it("resolves a story with both kinds pending to the dialog the board renders", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "Both at once")],
      humans: [rawHuman("h1", "s1", "Answer me")],
      approvals: [rawApproval("a1", "s1")],
    });

    await screen.findByText("Both at once");
    await user.click(marker("Both at once"));

    expect(inputDialog()).toBeInTheDocument();
    expect(approvalDialog()).toBeNull();
  });

  // A question raised outside a run, or an approval whose story was deleted,
  // has no card. It must stay reachable through the banner rather than
  // vanishing or marking a card at random.
  it("leaves a request with no story to the banner rather than marking a card", async () => {
    renderBoard({
      stories: [rawStory("s1", "Unrelated")],
      humans: [rawHuman("h1", null)],
    });

    await screen.findByText("Unrelated");
    expect(
      within(screen.getByLabelText(/: Unrelated$/)).queryByRole("button", {
        name: /waiting on you/i,
      }),
    ).toBeNull();
    expect(bannerButton("input")).toHaveTextContent(/respond/i);
  });
});

describe("BoardPage reaches a request nothing else can", () => {
  // The sharp end of the defect: with *every* request dismissed there is no
  // "first non-dismissed" one left, so the old rule had nothing to return and
  // no marker or button could reach any of them. The runs stayed blocked.
  it("opens a card's question when every request has been dismissed", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "First"), rawStory("s2", "Second")],
      humans: [
        rawHuman("h1", "s1", "Question for first"),
        rawHuman("h2", "s2", "Question for second"),
      ],
    });

    await screen.findByText("Second");

    // Dismiss both, which is what a user does when the dialogs are in the way.
    await user.click(marker("First"));
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    await user.click(marker("Second"));
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(inputDialog()).toBeNull();

    // Now go back for one specific card.
    await user.click(marker("Second"));
    expect(inputDialog()).toBeInTheDocument();
    expect(screen.getByText("Question for second")).toBeInTheDocument();
  });

  it("opens an approval when every request has been dismissed", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "Wants to write")],
      approvals: [rawApproval("a1", "s1")],
    });

    await screen.findByText("Wants to write");
    await user.click(marker("Wants to write"));
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(approvalDialog()).toBeNull();

    await user.click(marker("Wants to write"));
    expect(approvalDialog()).toBeInTheDocument();
  });
});

describe("BoardPage banners reach a dismissed request", () => {
  // The defect. Both banner actions read "the first non-dismissed request",
  // which is `null` once everything has been dismissed — precisely when you
  // would press the button. The run stayed blocked with no reachable UI.
  it("reopens a question after it has been dismissed", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "Blocked work")],
      humans: [rawHuman("h1", "s1", "Still waiting")],
    });

    await screen.findByText("Blocked work");

    await user.click(marker("Blocked work"));
    expect(inputDialog()).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(inputDialog()).toBeNull();

    // The banner is the only way back, and it has to work here.
    await user.click(bannerButton("input"));
    expect(inputDialog()).toBeInTheDocument();
    expect(screen.getByText("Still waiting")).toBeInTheDocument();
  });

  it("reopens an approval after it has been dismissed", async () => {
    const user = userEvent.setup();
    renderBoard({
      stories: [rawStory("s1", "Wants to write")],
      approvals: [rawApproval("a1", "s1")],
    });

    await screen.findByText("Wants to write");

    await user.click(marker("Wants to write"));
    expect(approvalDialog()).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(approvalDialog()).toBeNull();

    // Before the fix this banner had no button at all: dismiss once and the
    // run was blocked forever.
    await user.click(bannerButton("approval"));
    expect(approvalDialog()).toBeInTheDocument();
  });
});

describe("BoardPage answers a request", () => {
  it("sends the response through and closes the dialog", async () => {
    const user = userEvent.setup();
    // Stateful on purpose: the dialog closes because the backend stops
    // reporting the request, not because the page decided to hide it. A fixture
    // that keeps returning a pending question would leave the board correctly
    // reopening it, and the test would be asserting the wrong mechanism.
    let pending = [rawHuman("h1", "s1")];
    renderBoard({
      stories: [rawStory("s1", "Blocked work")],
      humans: pending,
    });
    tauriMock.handleAll({
      get_pending_human_requests: () => pending,
      respond_to_human_request: () => {
        pending = [];
        return null;
      },
    });

    await screen.findByText("Blocked work");
    await user.click(marker("Blocked work"));

    await user.type(screen.getByRole("textbox", { name: /response/i }), "go ahead");
    await user.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(inputDialog()).toBeNull());
    expect(tauriMock.calls("respond_to_human_request")).toEqual([
      { storyId: "human-h1", response: "go ahead" },
    ]);
  });
});
