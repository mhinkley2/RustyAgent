import { useState } from "react";
import { MessageSquare, SendHorizonal } from "lucide-react";
import type { HumanRequest } from "../../types/human";

// ---------------------------------------------------------------------------
// HumanInputDialog
//
// Shown when there are pending human-type stories.  The user reads the
// agent's question and submits a text reply.
// ---------------------------------------------------------------------------

interface HumanInputDialogProps {
  request: HumanRequest;
  onSubmit: (storyId: string, response: string) => Promise<void>;
  onDismiss: () => void;
  pendingApprovalCount?: number;
}

export function HumanInputDialog({ request, onSubmit, onDismiss, pendingApprovalCount = 0 }: HumanInputDialogProps) {
  const [response, setResponse] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit() {
    const trimmed = response.trim();
    if (!trimmed) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(request.storyId, trimmed);
    } catch (e) {
      setError(String(e));
      setSubmitting(false);
    }
  }

  return (
    <div className="hid-overlay" role="dialog" aria-modal="true" aria-label="Agent is asking for input">
      <div className="hid-modal">
        {/* ── Header ─────────────────────────────────────────────── */}
        <div className="hid-header">
          <MessageSquare size={16} className="hid-header__icon" />
          <h2 className="hid-header__title">Agent needs your input</h2>
        </div>

        {/* ── Story context ───────────────────────────────────────── */}
        <p className="hid-story-name">{request.storyTitle}</p>

        {/* ── Agent's question ─────────────────────────────────────── */}
        <div className="hid-question-block">
          <span className="hid-question-label">Question from agent</span>
          <p className="hid-question-text">{request.question ?? "(no question provided)"}</p>
        </div>

        {/* ── User response ────────────────────────────────────────── */}
        <label className="hid-response-label" htmlFor="hid-response">
          Your response
        </label>
        <textarea
          id="hid-response"
          className="hid-response-textarea"
          value={response}
          onChange={e => setResponse(e.target.value)}
          placeholder="Type your reply here…"
          rows={5}
          disabled={submitting}
          autoFocus
          onKeyDown={e => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") handleSubmit();
          }}
        />
        <p className="hid-hint">⌘ Enter to submit</p>

        {/* ── Error ────────────────────────────────────────────────── */}
        {error && <p className="hid-error">{error}</p>}

        {/* ── Approval notice ──────────────────────────────────────── */}
        {pendingApprovalCount > 0 && (
          <p className="hid-approval-notice">
            ⚠ {pendingApprovalCount} tool approval{pendingApprovalCount !== 1 ? "s are" : " is"} also waiting.
          </p>
        )}

        {/* ── Actions ──────────────────────────────────────────────── */}
        <div className="hid-actions">
          <button className="btn btn--ghost btn--sm" onClick={onDismiss} disabled={submitting}>
            Dismiss
          </button>
          <button
            className="btn btn--primary btn--sm"
            onClick={handleSubmit}
            disabled={submitting || !response.trim()}
          >
            <SendHorizonal size={13} />
            {submitting ? "Sending…" : "Send reply"}
          </button>
        </div>
      </div>
    </div>
  );
}
