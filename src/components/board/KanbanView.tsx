import { useState, useCallback, useEffect, useRef } from "react";
import { GitBranch } from "lucide-react";
import {
  DndContext,
  DragOverlay,
  closestCorners,
  PointerSensor,
  KeyboardSensor,
  useSensor,
  useSensors,
  useDroppable,
  type DragStartEvent,
  type DragOverEvent,
  type DragEndEvent,
  type UniqueIdentifier,
} from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { Story, StoryStatus, StoryPriority } from "../../types/board";
import { KANBAN_COLUMNS } from "../../types/board";

// ---------------------------------------------------------------------------
// Priority helpers
// ---------------------------------------------------------------------------

const PRIORITY_COLORS: Record<StoryPriority, string> = {
  critical: "var(--error)",
  high:     "var(--warning)",
  medium:   "var(--info)",
  low:      "var(--text-muted)",
};

const PRIORITY_LABELS: Record<StoryPriority, string> = {
  critical: "Critical",
  high:     "High",
  medium:   "Medium",
  low:      "Low",
};

// ---------------------------------------------------------------------------
// Recency formatting
// ---------------------------------------------------------------------------

function timeAgo(date: Date): string {
  const diffMs = Date.now() - date.getTime();
  const s = Math.round(diffMs / 1000);
  if (s < 60)  return "just now";
  const m = Math.round(s / 60);
  if (m < 60)  return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24)  return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

// ---------------------------------------------------------------------------
// StoryCard (pure visual – no native drag)
// ---------------------------------------------------------------------------

interface StoryCardProps {
  story: Story;
  onSelect: (story: Story) => void;
  isDragging?: boolean;
  /** Spread onto the drag-handle element (from useSortable). */
  dragProps?: React.HTMLAttributes<HTMLDivElement>;
}

export function StoryCard({ story, onSelect, isDragging, dragProps }: StoryCardProps) {
  return (
    <div
      className={`story-card story-card--${story.type}${isDragging ? " story-card--dragging" : ""}`}
      onClick={() => onSelect(story)}
      role="button"
      tabIndex={0}
      aria-label={`Story ${story.key}: ${story.title}`}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onSelect(story); }}
      {...dragProps}
    >
      <div className="story-card__top">
        <span
          className="story-card__priority-dot"
          style={{ background: PRIORITY_COLORS[story.priority] }}
          title={PRIORITY_LABELS[story.priority]}
          aria-label={PRIORITY_LABELS[story.priority]}
        />
        <span className="story-card__title">{story.title}</span>
        {story.type === "human" && (
          <span className="story-card__human-badge" title="Waiting for your input">⚑</span>
        )}
        {story.type === "pipeline" && (
          <span className="story-card__pipeline-badge" title="Pipeline">
            <GitBranch size={11} />
          </span>
        )}
      </div>
      <div className="story-card__meta">
        <span className="story-card__assignee">
          {story.assignee ?? "Unassigned"}
        </span>
      </div>
      <div className="story-card__footer">
        <span className="story-card__key">{story.key}</span>
        <span className="story-card__sep">·</span>
        <span className="story-card__type">{story.type}</span>
        <span className="story-card__sep">·</span>
        <span className="story-card__time">{timeAgo(story.updatedAt)}</span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// SortableCard — wraps StoryCard with useSortable
// ---------------------------------------------------------------------------

function SortableCard({ story, onSelect }: { story: Story; onSelect: (s: Story) => void }) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: story.id });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.35 : 1,
    position: "relative",
    zIndex: isDragging ? 1 : undefined,
  };

  return (
    <div ref={setNodeRef} style={style}>
      <StoryCard
        story={story}
        onSelect={onSelect}
        isDragging={isDragging}
        dragProps={{ ...attributes, ...listeners }}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// KanbanColumn
// ---------------------------------------------------------------------------

interface KanbanColumnProps {
  status: StoryStatus;
  label: string;
  stories: Story[];
  isDragOver: boolean;
  onSelect: (story: Story) => void;
  emptyMessage?: string;
}

function KanbanColumn({ status, label, stories, isDragOver, onSelect, emptyMessage }: KanbanColumnProps) {
  const { setNodeRef } = useDroppable({ id: status });
  const ids = stories.map(s => s.id);

  return (
    <div
      className={`kb-col${isDragOver ? " kb-col--drag-over" : ""}`}
      data-status={status}
    >
      <div className="kb-col__header">
        <span className="kb-col__label">{label}</span>
        <span className="kb-col__count">{stories.length}</span>
      </div>
      <div className="kb-col__cards" ref={setNodeRef}>
        <SortableContext items={ids} strategy={verticalListSortingStrategy}>
          {stories.length === 0 ? (
            <div className="kb-col__empty" aria-hidden>{emptyMessage ?? "Drop here"}</div>
          ) : (
            stories.map(s => (
              <SortableCard key={s.id} story={s} onSelect={onSelect} />
            ))
          )}
        </SortableContext>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// KanbanView
// ---------------------------------------------------------------------------

type ColMap = Record<StoryStatus, Story[]>;

function buildColMap(stories: Story[]): ColMap {
  const colMap = {} as ColMap;
  for (const { status } of KANBAN_COLUMNS) {
    colMap[status] = stories.filter(s => s.status === status && s.type !== "human");
  }
  return colMap;
}

function isColumnId(id: UniqueIdentifier): id is StoryStatus {
  return KANBAN_COLUMNS.some(c => c.status === id);
}

interface KanbanViewProps {
  stories: Story[];
  onSelect: (story: Story) => void;
  onMove: (storyId: string, newStatus: StoryStatus) => Promise<void>;
  onReorder: (updates: { id: string; sortOrder: number }[]) => Promise<void>;
}

const EMPTY_MESSAGES: Record<StoryStatus, string> = {
  backlog:     "No backlog stories",
  ready:       "Nothing queued yet",
  in_progress: "No active runs",
  blocked:     "Nothing blocked",
  review:      "Nothing in review",
  done:        "No completed stories",
};

export function KanbanView({ stories, onSelect, onMove, onReorder }: KanbanViewProps) {
  // Local column map — drives rendering during and after drags
  const [colMap, setColMap] = useState<ColMap>(() => buildColMap(stories));
  const activeIdRef = useRef<string | null>(null);
  const [activeId, setActiveId] = useState<UniqueIdentifier | null>(null);
  const [overColId, setOverColId] = useState<StoryStatus | null>(null);
  // Track the original column at drag-start for cross-column detection
  const originalColRef = useRef<StoryStatus | null>(null);

  // Sync colMap from props whenever not actively dragging
  useEffect(() => {
    if (!activeIdRef.current) {
      setColMap(buildColMap(stories));
    }
  }, [stories]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const findColOf = useCallback((id: UniqueIdentifier, map: ColMap): StoryStatus | null => {
    for (const { status } of KANBAN_COLUMNS) {
      if (map[status].some(s => s.id === id)) return status;
    }
    return null;
  }, []);

  function handleDragStart({ active }: DragStartEvent) {
    const col = findColOf(active.id, colMap);
    activeIdRef.current = active.id as string;
    originalColRef.current = col;
    setActiveId(active.id);
    setOverColId(col);
  }

  function handleDragOver({ active, over }: DragOverEvent) {
    if (!over) return;
    const activeCol = findColOf(active.id, colMap);
    const overCol = isColumnId(over.id) ? over.id : findColOf(over.id, colMap);
    setOverColId(overCol);
    if (!activeCol || !overCol || activeCol === overCol) return;

    // Move the active card to the column it's hovering over (visual feedback)
    setColMap(prev => {
      const activeStory = prev[activeCol].find(s => s.id === active.id);
      if (!activeStory) return prev;
      const overItems = prev[overCol];
      const insertAt = isColumnId(over.id)
        ? overItems.length
        : overItems.findIndex(s => s.id === over.id);
      const newOverItems = [...overItems];
      newOverItems.splice(insertAt >= 0 ? insertAt : newOverItems.length, 0, activeStory);
      return {
        ...prev,
        [activeCol]: prev[activeCol].filter(s => s.id !== active.id),
        [overCol]: newOverItems,
      };
    });
  }

  async function handleDragEnd({ active, over }: DragEndEvent) {
    activeIdRef.current = null;
    setActiveId(null);
    setOverColId(null);

    if (!over) {
      // Cancelled — restore from props
      setColMap(buildColMap(stories));
      originalColRef.current = null;
      return;
    }

    const currentColMap = colMap; // capture before any setColMap
    const currentCol = findColOf(active.id, currentColMap);
    if (!currentCol) {
      originalColRef.current = null;
      return;
    }

    let finalItems = currentColMap[currentCol];

    // Within-column reorder: apply arrayMove if over a sibling card
    if (!isColumnId(over.id) && over.id !== active.id) {
      const oldIdx = finalItems.findIndex(s => s.id === active.id);
      const newIdx = finalItems.findIndex(s => s.id === over.id);
      if (oldIdx !== -1 && newIdx !== -1 && oldIdx !== newIdx) {
        finalItems = arrayMove(finalItems, oldIdx, newIdx);
        setColMap(prev => ({ ...prev, [currentCol]: finalItems }));
      }
    }

    const isCrossColumn = originalColRef.current && originalColRef.current !== currentCol;
    originalColRef.current = null;

    // Persist cross-column status change
    if (isCrossColumn) {
      await onMove(active.id as string, currentCol);
    }

    // Persist column order
    if (finalItems.length > 0) {
      const updates = finalItems.map((s, i) => ({ id: s.id, sortOrder: i }));
      await onReorder(updates);
    }
  }

  // Active story for DragOverlay
  const activeStory = activeId
    ? stories.find(s => s.id === activeId) ?? null
    : null;

  const humanStories = stories.filter(s => s.type === "human" && s.status !== "done");

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={handleDragStart}
      onDragOver={handleDragOver}
      onDragEnd={handleDragEnd}
    >
      <div className="kb">
        {/* Main columns */}
        <div className="kb__board">
          {KANBAN_COLUMNS.map(({ status, label }) => (
            <KanbanColumn
              key={status}
              status={status}
              label={label}
              stories={colMap[status]}
              isDragOver={overColId === status && activeId !== null}
              onSelect={onSelect}
              emptyMessage={EMPTY_MESSAGES[status]}
            />
          ))}
        </div>

        {/* Human stories lane */}
        {humanStories.length > 0 && (
          <div className="kb__human-lane">
            <span className="kb__human-lane-label">★ Needs your input</span>
            <div className="kb__human-cards">
              {humanStories.map(s => (
                <StoryCard
                  key={s.id}
                  story={s}
                  onSelect={onSelect}
                />
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Ghost card while dragging */}
      <DragOverlay dropAnimation={null}>
        {activeStory && (
          <div style={{ opacity: 0.9, transform: "rotate(2deg)", pointerEvents: "none" }}>
            <StoryCard story={activeStory} onSelect={() => {}} isDragging />
          </div>
        )}
      </DragOverlay>
    </DndContext>
  );
}
