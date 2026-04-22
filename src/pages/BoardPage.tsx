import { useState, useMemo, useEffect } from "react";
import { LayoutGrid, List, MessageSquare, ShieldAlert } from "lucide-react";

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
  const { stories, createStory, updateStory, deleteStory, reorderStories } = useStories();
  const { profiles: agents } = useAgents();
  const {
    humanRequests,
    approvalRequests,
    refresh: refreshHuman,
    respondToHuman,
    decideApproval,
  } = useHumanRequests();

  const [view, setView] = useState<View>("kanban");
  const [selectedStory, setSelectedStory] = useState<Story | null>(null);
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

  const handleMove = async (storyId: string, newStatus: Story["status"]) => {
    await updateStory(storyId, { status: newStatus });
    // Keep detail panel in sync
    setSelectedStory(prev =>
      prev?.id === storyId ? { ...prev, status: newStatus } : prev
    );
  };

  const handleReorder = async (updates: { id: string; sortOrder: number }[]) => {
    await reorderStories(updates);
  };

  const handleSelect = (story: Story) => setSelectedStory(story);
  const handleClosePanel = () => setSelectedStory(null);

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
    if (selectedStory?.id === deleteTarget.id) setSelectedStory(null);
    setDeleteTarget(null);
  };

  return (
    <div className="board-page">
      <PageHeader title="Board" ctaLabel="New Story" onCta={openCreate}>
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
          />
        ) : (
          <ListView
            stories={filteredStories}
            onSelect={handleSelect}
            onDeleteStories={async (ids) => {
              for (const id of ids) await deleteStory(id);
              if (selectedStory && ids.includes(selectedStory.id)) setSelectedStory(null);
            }}
          />
        )}
      </div>

      <StoryDetailPanel
        story={selectedStory}
        onClose={handleClosePanel}
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
