// ---------------------------------------------------------------------------
// Board / Story shared types
// ---------------------------------------------------------------------------

import type { RunStatus } from "./runs";

export type StoryStatus =
  | "backlog"
  | "ready"
  | "in_progress"
  | "blocked"
  | "review"
  | "done";

export type StoryPriority = "critical" | "high" | "medium" | "low";

export type StoryType = "task" | "human" | "pipeline";

// ---------------------------------------------------------------------------
// Pipeline progress types — mirrored from pipeline::PipelineProgress
// ---------------------------------------------------------------------------

export type StepStatus = "pending" | "running" | "done" | "failed";
export type PipelineStatus = "running" | "done" | "failed" | "cancelled";
export type PipelineMode = "sequential" | "parallel";

export interface StepProgress {
  index: number;
  label: string;
  storyId: string;
  agentId: string;
  runId: string | null;
  status: StepStatus;
}

export interface PipelineProgress {
  pipelineRunId: string;
  storyId: string;
  mode: PipelineMode;
  status: PipelineStatus;
  steps: StepProgress[];
}

export interface Story {
  id: string;
  key?: string;
  title: string;
  status: StoryStatus;
  priority: StoryPriority;
  type: StoryType;
  /** UUID of the assigned agent profile (for edit form). */
  assignedAgentId?: string;
  /** Display name of the assigned agent (resolved from JOIN). */
  assignee?: string;
  labels: string[];
  requiresApproval: boolean;
  trackHistory: boolean;
  sortOrder: number;
  createdAt: Date;
  updatedAt: Date;
  description?: string;
  /**
   * The most recent run against this story, absent if it has never run.
   *
   * Joined into `get_stories` rather than fetched per card — the board renders
   * every story at once.
   */
  latestRun?: StoryLatestRun;
}

/**
 * What the board shows about a story's most recent run.
 *
 * `status` is the **run** vocabulary from `types/runs.ts`, which legitimately
 * contains `failed` and is not the story vocabulary. The shape this replaced
 * invented a third spelling — `success` / `failure` — that matched neither the
 * column nor `RunStatus`, and it only ever held mock data, so nothing caught
 * it.
 *
 * There is no step total: an agent loop runs until it finishes or hits
 * `max_iterations`, so `iterationCount` is a count, not a fraction. The old
 * shape's `stepsCompleted`/`stepsTotal` described a progress bar the data
 * cannot support.
 */
export interface StoryLatestRun {
  id: string;
  status: RunStatus;
  startedAt: Date;
  /** Absent while the run is still going. */
  finishedAt?: Date;
  /** Iterations entered so far. */
  iterationCount: number;
  inputTokens: number;
  outputTokens: number;
  estimatedCostUsd: number;
}

export const KANBAN_COLUMNS: { status: StoryStatus; label: string }[] = [
  { status: "backlog",     label: "Backlog" },
  { status: "ready",       label: "Ready" },
  { status: "in_progress", label: "In Progress" },
  { status: "blocked",     label: "Blocked" },
  { status: "review",      label: "Review" },
  { status: "done",        label: "Done" },
];

// ---------------------------------------------------------------------------
// Mock data — replaced by real IPC calls once the backend is wired
// ---------------------------------------------------------------------------

export const MOCK_STORIES: Story[] = [
  {
    id: "1", key: "#1", title: "Research competitor pricing", status: "in_progress",
    priority: "high", type: "task", assignee: "GPT-4o Agent", requiresApproval: false, trackHistory: true, sortOrder: 0,
    labels: ["research", "phase-1"], createdAt: new Date(Date.now() - 86400_000 * 5), updatedAt: new Date(Date.now() - 3600_000),
    description: "Research and summarise the pricing structures of our top 5 competitors.",
    latestRun: { id: "mock-run-1", status: "running", startedAt: new Date(Date.now() - 900_000), iterationCount: 3, inputTokens: 2341, outputTokens: 512, estimatedCostUsd: 0.04 },
  },
  {
    id: "2", key: "#2", title: "Write intro blog post", status: "ready",
    priority: "medium", type: "task", assignee: "Claude Agent", requiresApproval: false, trackHistory: true, sortOrder: 1,
    labels: ["content"], createdAt: new Date(Date.now() - 86400_000 * 3), updatedAt: new Date(Date.now() - 7200_000),
    description: "Write a 600-word intro blog post about the product launch.",
  },
  {
    id: "3", key: "#3", title: "Review sales deck copy", status: "backlog",
    priority: "low", type: "task", requiresApproval: false, trackHistory: true, sortOrder: 2,
    labels: [], createdAt: new Date(Date.now() - 86400_000 * 7), updatedAt: new Date(Date.now() - 86400_000),
  },
  {
    id: "4", key: "#4", title: "Approve outreach email draft", status: "backlog",
    priority: "critical", type: "human", requiresApproval: true, trackHistory: true, sortOrder: 3,
    labels: ["urgent"], createdAt: new Date(Date.now() - 86400_000 * 2), updatedAt: new Date(Date.now() - 1800_000),
    description: "The agent needs you to approve the draft outreach email before sending.",
  },
  {
    id: "5", key: "#5", title: "Analyse Q1 sales data", status: "done",
    priority: "high", type: "task", assignee: "Research Agent", requiresApproval: false, trackHistory: true, sortOrder: 4,
    labels: ["analytics", "phase-1"], createdAt: new Date(Date.now() - 86400_000 * 10), updatedAt: new Date(Date.now() - 172800_000),
    latestRun: { id: "mock-run-5", status: "done", startedAt: new Date(Date.now() - 172800_000 - 720_000), finishedAt: new Date(Date.now() - 172800_000), iterationCount: 12, inputTokens: 8401, outputTokens: 1200, estimatedCostUsd: 0.14 },
  },
  {
    id: "6", key: "#6", title: "Generate product screenshots", status: "blocked",
    priority: "medium", type: "task", assignee: "GPT-4o Agent", requiresApproval: false, trackHistory: true, sortOrder: 5,
    labels: ["marketing"], createdAt: new Date(Date.now() - 86400_000 * 4), updatedAt: new Date(Date.now() - 10800_000),
    description: "Blocked: missing design assets from the design team.",
  },
  {
    id: "7", key: "#7", title: "Set up email automation pipeline", status: "review",
    priority: "high", type: "pipeline", assignee: "Automation Agent", requiresApproval: false, trackHistory: false, sortOrder: 6,
    labels: ["automation"], createdAt: new Date(Date.now() - 86400_000 * 6), updatedAt: new Date(Date.now() - 5400_000),
  },
  {
    id: "8", key: "#8", title: "Which pricing tier to focus on?", status: "backlog",
    priority: "critical", type: "human", requiresApproval: true, trackHistory: true, sortOrder: 7,
    labels: ["urgent", "decision"], createdAt: new Date(Date.now() - 86400_000), updatedAt: new Date(Date.now() - 900_000),
    description: "The agent is waiting for your decision: Basic or Pro tier focus?",
  },
];
