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
import { ChevronDown, ChevronRight } from "lucide-react";
import type { Story, StoryStatus } from "../../types/board";
import { rollbackTarget, type ColMap as RollbackColMap } from "./dragRollback";
import { KANBAN_COLUMNS } from "../../types/board";
import { nextUpIds } from "./queue";
import { placeInFullColumn } from "./reorder";
import { useCollapsedColumns } from "./collapsedColumns";
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
  /** Render at most this many cards until the user asks for the rest. */
  cardLimit?: number;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  /** The card currently being dragged, anywhere on the board. */
  activeId: UniqueIdentifier | null;
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
  cardLimit,
  collapsed,
  onToggleCollapsed,
  activeId,
}: KanbanColumnProps) {
  const { setNodeRef } = useDroppable({ id: status });

  /**
   * Whether the column is showing everything it holds.
   *
   * Reset whenever the cap stops applying, so a column that shrinks back under
   * the limit does not keep an expander with nothing behind it.
   */
  const [expanded, setExpanded] = useState(false);
  const overLimit = cardLimit !== undefined && stories.length > cardLimit;
  useEffect(() => {
    if (!overLimit) setExpanded(false);
  }, [overLimit]);

  const capped = overLimit && !expanded ? stories.slice(0, cardLimit) : stories;

  /**
   * The cap never hides the card being dragged.
   *
   * `handleDragOver` appends a card entering a column at the end of the list,
   * which for a capped column is past the cap — so the card the pointer is
   * holding would unmount, leaving nothing rendered where the drop is about to
   * land and no placeholder to aim at. It reappears at the top on the next
   * refetch, because Done reads most-recent-first, but the drag itself looked
   * like the card had been swallowed.
   */
  const visible =
    activeId !== null && !capped.some(s => s.id === activeId)
      ? [...capped, ...stories.filter(s => s.id === activeId)]
      : capped;

  const hiddenCount = stories.length - visible.length;

  // Only the rendered cards, or dnd-kit is tracking sortables that have no
  // node — the cap is a rendering limit, not a change to what the column holds.
  const ids = visible.map(s => s.id);

  // Only Ready is a queue. A card in Backlog or Review is not next for
  // anybody, and marking one would say something untrue.
  const nextUp = useMemo(
    () => (status === "ready" ? nextUpIds(stories) : new Set<string>()),
    [status, stories],
  );

  const headerToggle = (
    <button
      type="button"
      className="kb-col__collapse"
      aria-expanded={!collapsed}
      aria-label={`${collapsed ? "Expand" : "Collapse"} ${label} column`}
      onClick={onToggleCollapsed}
    >
      {collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
    </button>
  );

  if (collapsed) {
    return (
      <div
        className={`kb-col kb-col--collapsed${isDragOver ? " kb-col--drag-over" : ""}`}
        data-status={status}
      >
        {/*
          Still a drop target. Collapsing Done to get it out of the way and
          then being unable to drop a finished card into it is exactly the
          workflow the collapse is for, so the droppable ref goes on the
          collapsed body too — there is just no card list under it.
        */}
        <div className="kb-col__header" ref={setNodeRef}>
          {headerToggle}
          <span className="kb-col__label">{label}</span>
          <span className="kb-col__count">{stories.length}</span>
        </div>
      </div>
    );
  }

  return (
    <div
      className={`kb-col${isDragOver ? " kb-col--drag-over" : ""}`}
      data-status={status}
    >
      <div className="kb-col__header">
        {headerToggle}
        <span className="kb-col__label">{label}</span>
        <span className="kb-col__count">{stories.length}</span>
      </div>
      <div className="kb-col__cards" ref={setNodeRef}>
        <SortableContext items={ids} strategy={verticalListSortingStrategy}>
          {stories.length === 0 ? (
            <div className="kb-col__empty" aria-hidden>{emptyMessage ?? "Drop here"}</div>
          ) : (
            visible.map(s => (
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
        {hiddenCount > 0 && (
          <button type="button" className="kb-col__more" onClick={() => setExpanded(true)}>
            Show {hiddenCount} more
          </button>
        )}
        {expanded && overLimit && (
          <button type="button" className="kb-col__more" onClick={() => setExpanded(false)}>
            Show fewer
          </button>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// KanbanView
// ---------------------------------------------------------------------------

// The board as the columns render it. Defined beside the rollback rule that
// also reasons over it, so the two cannot drift apart.
type ColMap = RollbackColMap;

/**
 * One column's cards, in the order that column should read in.
 *
 * Every column arrives in board order — `db::story_status::queue_order_sql`,
 * which is `priority_rank ASC, sort_order ASC, created_at ASC`. That is the
 * right order for a queue: priority outranks manual position, and the drag
 * settles ties within a band.
 *
 * Done is not a queue. Nothing schedules from it, so its `sort_order` is
 * inert, and board order there means "highest priority, then oldest" — so the
 * story you just finished sorts *below* every ancient `critical` one. Capping
 * that list would hide a completion the moment it happened, which is the
 * opposite of what a capped Done column is for.
 *
 * So Done reads most-recently-touched first, and the cap takes the front of
 * that. `updated_at` is bumped by every write including the status change
 * (`commands::stories`, `updated_at = {NOW_ISO8601}`), so a card lands at the
 * top of Done at the moment it is dropped there.
 */
function columnOrder(status: StoryStatus, stories: Story[]): Story[] {
  if (status !== "done") return stories;
  return [...stories].sort((a, b) => b.updatedAt.getTime() - a.updatedAt.getTime());
}

function buildColMap(stories: Story[]): ColMap {
  const colMap = {} as ColMap;
  for (const { status } of KANBAN_COLUMNS) {
    colMap[status] = columnOrder(
      status,
      stories.filter(s => s.status === status && s.type !== "human"),
    );
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
  /**
   * Every story on the board, before filtering — used only to persist order.
   *
   * `stories` is what the columns draw, and with a filter active that is a
   * subset. Numbering the drawn column `0..n` would give a hidden card and a
   * visible card the same `sortOrder`, so the column comes back scrambled once
   * the filter clears. Absent, or equal to `stories`, this changes nothing.
   */
  allStories?: Story[];
  /**
   * Which columns to draw. Absent means all of them.
   *
   * Fed by the status filter so the filter and the column layout agree, rather
   * than leaving five columns saying "Nothing here" because the user asked for
   * one status.
   */
  visibleStatuses?: StoryStatus[];
}

/**
 * How many Done cards render before the column offers the rest.
 *
 * Done only grows, and it is already the tallest thing on the board. Capping
 * it in the UI leaves the query alone — if board *load* ever gets slow the cap
 * belongs in `get_stories` instead, and it should move rather than be
 * duplicated, or there are two limits to keep in agreement.
 */
const DONE_CARD_LIMIT = 10;

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
  allStories,
  visibleStatuses,
}: KanbanViewProps) {
  // Local column map — drives rendering during and after drags
  const [colMap, setColMap] = useState<ColMap>(() => buildColMap(stories));
  const { collapsed, toggle: toggleCollapsed } = useCollapsedColumns();
  const columns = useMemo(
    () =>
      visibleStatuses && visibleStatuses.length > 0
        ? KANBAN_COLUMNS.filter(c => visibleStatuses.includes(c.status))
        : KANBAN_COLUMNS,
    [visibleStatuses],
  );
  const activeIdRef = useRef<string | null>(null);
  const [activeId, setActiveId] = useState<UniqueIdentifier | null>(null);
  const [overColId, setOverColId] = useState<StoryStatus | null>(null);
  // Track the original column at drag-start for cross-column detection
  const originalColRef = useRef<StoryStatus | null>(null);
  /**
   * The board exactly as it stood before this drag began.
   *
   * `handleDragOver` rearranges `colMap` live as the pointer moves, so by the
   * time a persist fails there is nothing left that remembers where the card
   * came from. Without this a rejected write left the card sitting in the
   * column the write had just failed to put it in: an error toast, and a board
   * that silently disagrees with the database until something else happens to
   * change the story list.
   */
  const beforeDragRef = useRef<ColMap | null>(null);

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
    beforeDragRef.current = colMap;
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
    beforeDragRef.current = null;
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
      beforeDragRef.current = null;
      return;
    }

    const currentColMap = colMap; // capture before any setColMap
    const beforeDrag = beforeDragRef.current;
    beforeDragRef.current = null;
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

    const crossColumn = Boolean(isCrossColumn);
    // `activeIdRef` is read at rejection time, not now: it is what says whether
    // the user has since picked up another card.
    const rollback = (failed: "move" | "reorder") => {
      const snapshot = rollbackTarget({
        failed,
        crossColumn,
        beforeDrag,
        beforeReorder: currentColMap,
        dragInFlight: Boolean(activeIdRef.current),
      });
      if (snapshot) setColMap(snapshot);
    };

    // Persist the cross-column status change.
    if (crossColumn) {
      try {
        await onMove(active.id as string, currentCol);
      } catch {
        rollback("move");
        return;
      }
    }

    // Persist the column order.
    //
    // Over the *unfiltered* column: `finalItems` is only what the filter left
    // visible, and numbering that `0..n` would collide with the `sortOrder` of
    // every card the filter hid. `placeInFullColumn` is a no-op when nothing
    // is hidden.
    const fullColumn = allStories
      ? columnOrder(
          currentCol,
          allStories.filter(s => s.status === currentCol && s.type !== "human"),
        )
      : finalItems;
    const persisted = placeInFullColumn(fullColumn, finalItems, active.id as string);

    if (persisted.length > 0) {
      const updates = persisted.map((s, i) => ({ id: s.id, sortOrder: i }));
      try {
        await onReorder(updates);
      } catch {
        rollback("reorder");
      }
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
          {columns.map(({ status, label }) => (
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
              cardLimit={status === "done" ? DONE_CARD_LIMIT : undefined}
              collapsed={collapsed.has(status)}
              onToggleCollapsed={() => toggleCollapsed(status)}
              activeId={activeId}
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
