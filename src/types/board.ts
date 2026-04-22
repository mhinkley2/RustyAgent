// ---------------------------------------------------------------------------
// Board / Story shared types
// ---------------------------------------------------------------------------

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
  // Latest run summary (if any)
  latestRun?: {
    status: "running" | "success" | "failure" | "cancelled";
    startedAt: Date;
    durationMs: number;
    stepsCompleted: number;
    stepsTotal: number;
    tokens: number;
    costUsd: number;
  };
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
    latestRun: { status: "running", startedAt: new Date(Date.now() - 900_000), durationMs: 0, stepsCompleted: 3, stepsTotal: 8, tokens: 2341, costUsd: 0.04 },
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
    latestRun: { status: "success", startedAt: new Date(Date.now() - 172800_000 - 720_000), durationMs: 720_000, stepsCompleted: 12, stepsTotal: 12, tokens: 8401, costUsd: 0.14 },
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
