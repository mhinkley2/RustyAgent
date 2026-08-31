import { useState, useMemo, useEffect, useCallback } from "react";
import { LayoutGrid, List, MessageSquare, RefreshCw, ShieldAlert } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { KanbanView } from "../components/board/KanbanView";
import { ListView } from "../components/board/ListView";
import { StoryDetailPanel } from "../components/board/StoryDetailPanel";
import { StoryForm } from "../components/board/StoryForm";
import { PageHeader } from "../components/board/PageHeader";
import { FilterBar, DEFAULT_FILTERS } from "../components/board/FilterBar";
import { attentionByStory, type StoryAttention } from "../components/board/attention";
import { assignmentInput } from "../components/board/assignment";
import {
  activeRequests,
  pruneDismissed,
  undismiss,
} from "../components/board/activeRequest";
import { HumanInputDialog } from "../components/board/HumanInputDialog";
import { ApprovalGateDialog } from "../components/board/ApprovalGateDialog";
import { ConfirmDialog } from "../components/forms";
import type { BoardFilters } from "../components/board/FilterBar";
import type { Story } from "../types/board";
import { useStories } from "../hooks/useStories";
import { useAgents } from "../hooks/useAgents";
import { useHumanRequests, requestDesktopNotification } from "../hooks/useHumanRequests";

type View = "kanban" | "list";

export default function BoardPage() {
  const navigate = useNavigate();
  const {
    stories,
    refresh,
    lastFetchedAt,
    pauseAutoRefresh,
    resumeAutoRefresh,
    createStory,
    updateStory,
    deleteStory,
    reorderStories,
  } = useStories();
  const { profiles: agents } = useAgents();
  const {
    humanRequests,
    approvalRequests,
    refresh: refreshHuman,
    respondToHuman,
    decideApproval,
  } = useHumanRequests();

  const [view, setView] = useState<View>("kanban");
  /**
   * The open detail panel is keyed by id, not by a story object.
   *
   * Holding the object meant the panel showed the snapshot it opened with and
   * had to be patched by hand on every change — which stops working entirely
   * once the board refreshes itself underneath it.
   */
  const [selectedStoryId, setSelectedStoryId] = useState<string | null>(null);
  const [filters, setFilters] = useState<BoardFilters>(DEFAULT_FILTERS);
  const [formOpen, setFormOpen] = useState(false);
  const [editStory, setEditStory] = useState<Story | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Story | null>(null);
  const [dismissedHumanIds, setDismissedHumanIds] = useState<Set<string>>(new Set());
  const [dismissedApprovalIds, setDismissedApprovalIds] = useState<Set<string>>(new Set());

  // Show desktop notifications when new human requests arrive
  useEffect(() => {
    for (const req of humanRequests) {
      if (!dismissedHumanIds.has(req.id)) {
        requestDesktopNotification(
          "Agent needs your input",
          req.storyTitle || req.question || "An agent is waiting for a response."
        );
      }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [humanRequests.length]);

  /**
   * The one request the user explicitly asked to see.
   *
   * Everything used to be "the first non-dismissed request", which is exactly
   * what made a dismissed request unreachable: dismiss them all and there is no
   * first non-dismissed one left for a button to act on. A run could be left
   * blocked with no UI that could reopen it.
   */
  const [focusedRequestId, setFocusedRequestId] = useState<string | null>(null);

  const { human: activeHumanRequest, approval: activeApprovalRequest } = useMemo(
    () =>
      activeRequests(
        humanRequests,
        approvalRequests,
        dismissedHumanIds,
        dismissedApprovalIds,
        focusedRequestId,
      ),
    [
      humanRequests,
      approvalRequests,
      dismissedHumanIds,
      dismissedApprovalIds,
      focusedRequestId,
    ],
  );

  /** Open one specific request, undoing any dismissal that is hiding it. */
  const openRequest = useCallback((id: string) => {
    setFocusedRequestId(id);
    setDismissedHumanIds(prev => undismiss(prev, id));
    setDismissedApprovalIds(prev => undismiss(prev, id));
  }, []);

  const closeRequest = useCallback(() => setFocusedRequestId(null), []);

  // Forget dismissals of requests that no longer exist, so the sets do not
  // grow for the life of the page remembering things that are gone.
  useEffect(() => {
    const liveHuman = new Set(humanRequests.map(r => r.id));
    const liveApproval = new Set(approvalRequests.map(r => r.id));
    setDismissedHumanIds(prev => pruneDismissed(prev, liveHuman));
    setDismissedApprovalIds(prev => pruneDismissed(prev, liveApproval));
  }, [humanRequests, approvalRequests]);

  /**
   * Which cards are blocking a person.
   *
   * The banners say how many; this is what says *which*, on the board itself,
   * without opening anything.
   */
  const attention = useMemo(
    () => attentionByStory(humanRequests, approvalRequests),
    [humanRequests, approvalRequests],
  );

  const handleAttention = useCallback(
    (a: StoryAttention) => openRequest(a.requestId),
    [openRequest],
  );

  /**
   * Assign one story.
   *
   * `updateStory` replaces the row in local state with what the write returned,
   * and the open panel is derived from that row rather than from a snapshot —
   * so Run Now and Start Pipeline stop being disabled without a refetch.
   */
  const handleAssign = useCallback(
    async (storyId: string, agentId: string | null) => {
      await updateStory(storyId, assignmentInput(agentId));
    },
    [updateStory],
  );

  /**
   * Assign a selection.
   *
   * Sequential rather than `Promise.all`: each write returns the whole story
   * and sets it into the same list, and firing them together would have several
   * responses racing to replace it.
   */
  const handleAssignMany = useCallback(
    async (ids: string[], agentId: string | null) => {
      for (const id of ids) await updateStory(id, assignmentInput(agentId));
    },
    [updateStory],
  );

  // All unique labels across all stories
  const availableLabels = useMemo(() => {
    const set = new Set<string>();
    for (const s of stories) s.labels.forEach(l => set.add(l));
    return [...set].sort();
  }, [stories]);

  // Apply filters
  const filteredStories = useMemo(() => {
    return stories.filter(s => {
      if (filters.quick === "mine" && !s.assignee) return false;
      if (filters.quick === "unassigned" && s.assignee) return false;
      if (filters.priorities.length > 0 && !filters.priorities.includes(s.priority)) return false;
      if (filters.types.length > 0 && !filters.types.includes(s.type)) return false;
      if (filters.labels.length > 0 && !filters.labels.some(l => s.labels.includes(l))) return false;
      return true;
    });
  }, [stories, filters]);

  /**
   * The story the open panel is showing, derived rather than stored.
   *
   * A story that disappears from the board — deleted, or filtered out by a
   * workspace switch — closes the panel by becoming `null` here, with nothing
   * to remember to do.
   */
  const selectedStory = useMemo(
    () => stories.find(s => s.id === selectedStoryId) ?? null,
    [stories, selectedStoryId],
  );

  const handleMove = async (storyId: string, newStatus: Story["status"]) => {
    await updateStory(storyId, { status: newStatus });
  };

  /**
   * How long ago the board was read, as a label.
   *
   * Re-derived on a five-second tick rather than every second: the number is
   * there to say "this is a live view and here is its age", and a board that
   * re-rendered once a second to move a digit would cost more than it tells.
   */
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(timer);
  }, []);

  const staleness = useMemo(() => {
    if (!lastFetchedAt) return "Refresh";
    const seconds = Math.max(0, Math.round((now - lastFetchedAt.getTime()) / 1000));
    if (seconds < 10) return "Updated just now";
    if (seconds < 60) return `Updated ${seconds}s ago`;
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `Updated ${minutes}m ago`;
    return `Updated ${Math.round(minutes / 60)}h ago`;
  }, [lastFetchedAt, now]);

  const handleReorder = async (updates: { id: string; sortOrder: number }[]) => {
    await reorderStories(updates);
  };

  const handleSelect = (story: Story) => setSelectedStoryId(story.id);
  const handleClosePanel = () => setSelectedStoryId(null);

  const openCreate = () => {
    setEditStory(null);
    setFormOpen(true);
  };
  const openEdit = (story: Story) => {
    setEditStory(story);
    setFormOpen(true);
  };
  const handleFormClose = () => setFormOpen(false);

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    await deleteStory(deleteTarget.id);
    if (selectedStoryId === deleteTarget.id) setSelectedStoryId(null);
    setDeleteTarget(null);
  };

  return (
    <div className="board-page">
      <PageHeader
        title="Board"
        cta={
          // `cta` replaces `ctaLabel`/`onCta` rather than sitting beside them,
          // so New Story lives here too.
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <button
              className="btn btn--ghost btn--sm"
              onClick={() => void refresh()}
              title="Refetch the board"
            >
              <RefreshCw size={14} />
              {staleness}
            </button>
            <button className="btn btn--primary btn--sm" onClick={openCreate}>
              New Story
            </button>
          </div>
        }
      >
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <FilterBar
            filters={filters}
            onChange={setFilters}
            availableLabels={availableLabels}
          />
          <div className="view-toggle" style={{ marginLeft: "auto", flexShrink: 0 }}>
            <button
              className={`view-toggle__btn${view === "kanban" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setView("kanban")}
              aria-label="Kanban view"
            >
              <LayoutGrid size={14} />
              Kanban
            </button>
            <button
              className={`view-toggle__btn${view === "list" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setView("list")}
              aria-label="List view"
            >
              <List size={14} />
              List
            </button>
          </div>
        </div>
      </PageHeader>

      {/* ── Human-in-the-loop banners ─────────────────────────────── */}
      {humanRequests.length > 0 && (
        <div className="hitl-banner hitl-banner--input">
          <MessageSquare size={14} className="hitl-banner__icon" />
          <span className="hitl-banner__text">
            {humanRequests.length === 1
              ? "An agent is waiting for your input."
              : `${humanRequests.length} agents are waiting for your input.`}
          </span>
          <button
            className="btn btn--primary btn--xs"
            // `[0]` is the oldest: both reads come back `created_at ASC`, so
            // this unblocks the run that has been stuck longest.
            //
            // Dismissed or not — the button has to work in exactly the case you
            // would reach for it, which is when you have dismissed everything
            // and want one back.
            onClick={() => openRequest(humanRequests[0].id)}
          >
            Respond
          </button>
        </div>
      )}
      {approvalRequests.length > 0 && (
        <div className="hitl-banner hitl-banner--approval">
          <ShieldAlert size={14} className="hitl-banner__icon" />
          <span className="hitl-banner__text">
            {approvalRequests.length === 1
              ? "A tool call is waiting for your approval."
              : `${approvalRequests.length} tool calls are waiting for approval.`}
          </span>
          <button
            className="btn btn--primary btn--xs"
            // Same rule as Respond above: oldest first, dismissed or not.
            onClick={() => openRequest(approvalRequests[0].id)}
          >
            Review
          </button>
        </div>
      )}

      <div className="board-page__content">
        {view === "kanban" ? (
          <KanbanView
            stories={filteredStories}
            onSelect={handleSelect}
            onMove={handleMove}
            onReorder={handleReorder}
            onDragActiveChange={(dragging) =>
              dragging ? pauseAutoRefresh() : resumeAutoRefresh()
            }
            attention={attention}
            onAttention={handleAttention}
            agents={agents}
            onAssign={handleAssign}
          />
        ) : (
          <ListView
            stories={filteredStories}
            onSelect={handleSelect}
            agents={agents}
            onAssignStories={handleAssignMany}
            onDeleteStories={async (ids) => {
              for (const id of ids) await deleteStory(id);
              if (selectedStoryId && ids.includes(selectedStoryId)) setSelectedStoryId(null);
            }}
          />
        )}
      </div>

      <StoryDetailPanel
        story={selectedStory}
        onClose={handleClosePanel}
        // The runs page already renders a live timeline for a single run, so
        // opening one is a navigation rather than another view to build.
        onOpenRun={(runId) => navigate(`/runs?runId=${runId}`)}
        onEdit={openEdit}
        onDelete={setDeleteTarget}
        agents={agents}
        onAssign={handleAssign}
        onRun={(story) => {
          // TODO: dispatch IPC run command
          console.info("Run story:", story.id);
        }}
      />

      <StoryForm
        open={formOpen}
        story={editStory}
        agents={agents}
        onClose={handleFormClose}
        onCreate={createStory}
        onUpdate={updateStory}
      />

      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete "${deleteTarget?.title}"?`}
        body="This will permanently delete the story and all its runs. This cannot be undone."
        confirmLabel="Delete Story"
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleDeleteConfirm}
      />

      {/* ── Human input dialog ────────────────────────────────────── */}
      {activeHumanRequest && (
        <HumanInputDialog
          request={activeHumanRequest}
          onSubmit={async (storyId, response) => {
            closeRequest();
            await respondToHuman(storyId, response);
            await refreshHuman();
          }}
          onDismiss={() => {
            closeRequest();
            setDismissedHumanIds(s => new Set([...s, activeHumanRequest.id]));
          }}
          pendingApprovalCount={activeApprovalRequest ? 1 : 0}
        />
      )}

      {/* ── Approval gate dialog ──────────────────────────────────── */}
      {activeApprovalRequest && !activeHumanRequest && (
        <ApprovalGateDialog
          request={activeApprovalRequest}
          onDecide={async (id, approved, reason) => {
            closeRequest();
            await decideApproval(id, approved, reason);
            await refreshHuman();
          }}
          onDismiss={() => {
            closeRequest();
            setDismissedApprovalIds(s => new Set([...s, activeApprovalRequest.id]));
          }}
        />
      )}
    </div>
  );
}
