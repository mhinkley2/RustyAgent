import { useState, useCallback, useEffect, useMemo, useRef } from "react";
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
import type { Story, StoryStatus } from "../../types/board";
import { KANBAN_COLUMNS } from "../../types/board";
import { nextUpIds } from "./queue";
import { StoryCard } from "./StoryCard";
import type { StoryAttention } from "./attention";
import type { AgentProfile } from "../../types/agent";

// ---------------------------------------------------------------------------
// SortableCard — wraps StoryCard with useSortable
// ---------------------------------------------------------------------------

function SortableCard({
  story,
  onSelect,
  isNextUp,
  attention,
  onAttention,
  agents,
  onAssign,
}: {
  story: Story;
  onSelect: (s: Story) => void;
  isNextUp?: boolean;
  attention?: StoryAttention;
  onAttention?: (attention: StoryAttention) => void;
  agents?: AgentProfile[];
  onAssign?: (storyId: string, agentId: string | null) => Promise<void> | void;
}) {
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
        isNextUp={isNextUp}
        attention={attention}
        onAttention={onAttention}
        agents={agents}
        onAssign={onAssign && ((agentId) => onAssign(story.id, agentId))}
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
  attention: Map<string, StoryAttention>;
  onAttention?: (attention: StoryAttention) => void;
  agents?: AgentProfile[];
  onAssign?: (storyId: string, agentId: string | null) => Promise<void> | void;
}

function KanbanColumn({
  status,
  label,
  stories,
  isDragOver,
  onSelect,
  emptyMessage,
  attention,
  onAttention,
  agents,
  onAssign,
}: KanbanColumnProps) {
  const { setNodeRef } = useDroppable({ id: status });
  const ids = stories.map(s => s.id);

  // Only Ready is a queue. A card in Backlog or Review is not next for
  // anybody, and marking one would say something untrue.
  const nextUp = useMemo(
    () => (status === "ready" ? nextUpIds(stories) : new Set<string>()),
    [status, stories],
  );

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
              <SortableCard
                key={s.id}
                story={s}
                onSelect={onSelect}
                isNextUp={nextUp.has(s.id)}
                attention={attention.get(s.id)}
                onAttention={onAttention}
                agents={agents}
                onAssign={onAssign}
              />
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
  /**
   * Called as a drag begins and ends.
   *
   * The board refetches itself on a timer and whenever another writer changes
   * a story. A refetch landing between a drop and the write that persists it
   * would replace the optimistic order with the pre-drop one, so the board
   * holds automatic refreshes for the duration of a drag.
   */
  onDragActiveChange?: (dragging: boolean) => void;
  /**
   * Which cards are blocking a person, keyed by story id.
   *
   * Passed in rather than derived here: the requests live on `BoardPage`
   * alongside the dialogs that resolve them, and the board should not have to
   * know how to fetch them in order to draw one chip.
   */
  attention?: Map<string, StoryAttention>;
  /** Open the dialog behind a card's marker. */
  onAttention?: (attention: StoryAttention) => void;
  /**
   * Profiles the cards' assignee slots can offer.
   *
   * Assignment is a precondition for everything that starts work, so it has to
   * be reachable where the stories are — not only inside the edit form.
   */
  agents?: AgentProfile[];
  /** Assign a story from its card. Absent leaves the assignee read-only. */
  onAssign?: (storyId: string, agentId: string | null) => Promise<void> | void;
}

const EMPTY_MESSAGES: Record<StoryStatus, string> = {
  backlog:     "No backlog stories",
  ready:       "Nothing queued yet",
  in_progress: "No active runs",
  blocked:     "Nothing blocked",
  review:      "Nothing in review",
  done:        "No completed stories",
};

const NO_ATTENTION: Map<string, StoryAttention> = new Map();

export function KanbanView({
  stories,
  onSelect,
  onMove,
  onReorder,
  onDragActiveChange,
  attention = NO_ATTENTION,
  onAttention,
  agents,
  onAssign,
}: KanbanViewProps) {
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
    onDragActiveChange?.(true);
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

  /**
   * A drag abandoned rather than dropped — Escape, or dnd-kit cancelling it.
   *
   * `onDragEnd` does not fire in that case, so without this the board would be
   * left with auto-refresh paused for good: one cancelled drag and the card
   * stops following the database, silently, with nothing to un-stick it.
   */
  function handleDragCancel() {
    onDragActiveChange?.(false);
    activeIdRef.current = null;
    setActiveId(null);
    setOverColId(null);
    originalColRef.current = null;
    // The columns were rearranged live as the pointer moved; put them back.
    setColMap(buildColMap(stories));
  }

  async function handleDragEnd({ active, over }: DragEndEvent) {
    // Released before the awaits below: the drop is decided here, and the
    // persist that follows is what a deferred refresh should land after.
    onDragActiveChange?.(false);
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
      onDragCancel={handleDragCancel}
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
              attention={attention}
              onAttention={onAttention}
              agents={agents}
              onAssign={onAssign}
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
