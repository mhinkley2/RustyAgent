import { useState, useMemo, useEffect } from "react";
import { LayoutGrid, List, MessageSquare, RefreshCw, ShieldAlert } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { KanbanView } from "../components/board/KanbanView";
import { ListView } from "../components/board/ListView";
import { StoryDetailPanel } from "../components/board/StoryDetailPanel";
import { StoryForm } from "../components/board/StoryForm";
import { PageHeader } from "../components/board/PageHeader";
import { FilterBar, DEFAULT_FILTERS } from "../components/board/FilterBar";
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

  // Active dialogs: pick the first non-dismissed pending request
  const activeHumanRequest = humanRequests.find(r => !dismissedHumanIds.has(r.id)) ?? null;
  const activeApprovalRequest = approvalRequests.find(r => !dismissedApprovalIds.has(r.id)) ?? null;

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
              onClick={() => {
                if (activeHumanRequest) {
                  setDismissedHumanIds(s => { const n = new Set(s); n.delete(activeHumanRequest.id); return n; });
                }
              }}
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
          />
        ) : (
          <ListView
            stories={filteredStories}
            onSelect={handleSelect}
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
            await respondToHuman(storyId, response);
            await refreshHuman();
          }}
          onDismiss={() =>
            setDismissedHumanIds(s => new Set([...s, activeHumanRequest.id]))
          }
          pendingApprovalCount={activeApprovalRequest ? 1 : 0}
        />
      )}

      {/* ── Approval gate dialog ──────────────────────────────────── */}
      {activeApprovalRequest && !activeHumanRequest && (
        <ApprovalGateDialog
          request={activeApprovalRequest}
          onDecide={async (id, approved, reason) => {
            await decideApproval(id, approved, reason);
            await refreshHuman();
          }}
          onDismiss={() =>
            setDismissedApprovalIds(s => new Set([...s, activeApprovalRequest.id]))
          }
        />
      )}
    </div>
  );
}
