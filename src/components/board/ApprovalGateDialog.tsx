import { useState } from "react";
import { ShieldAlert, Check, X } from "lucide-react";
import type { ApprovalRequest } from "../../types/human";

// ---------------------------------------------------------------------------
// ApprovalGateDialog
//
// Shown when a run is paused for user approval before executing a tool call.
// The user sees the tool name and its inputs and can approve or reject.
// ---------------------------------------------------------------------------

function tryPrettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

interface ApprovalGateDialogProps {
  request: ApprovalRequest;
  onDecide: (id: string, approved: boolean, rejectionReason?: string) => Promise<void>;
  onDismiss: () => void;
}

export function ApprovalGateDialog({ request, onDecide, onDismiss }: ApprovalGateDialogProps) {
  const [rejecting, setRejecting] = useState(false);
  const [rejectionReason, setRejectionReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const prettyInput = tryPrettyJson(request.toolInput);

  async function handleApprove() {
    setBusy(true);
    setError(null);
    try {
      await onDecide(request.id, true);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  async function handleReject() {
    setBusy(true);
    setError(null);
    try {
      await onDecide(request.id, false, rejectionReason.trim() || undefined);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="agd-overlay" role="dialog" aria-modal="true" aria-label="Approve tool execution">
      <div className="agd-modal">
        {/* ── Header ─────────────────────────────────────────────── */}
        <div className="agd-header">
          <ShieldAlert size={16} className="agd-header__icon" />
          <h2 className="agd-header__title">Approve tool execution?</h2>
        </div>

        {/* ── Story context ───────────────────────────────────────── */}
        {request.storyTitle && (
          <p className="agd-story-name">{request.storyTitle}</p>
        )}

        {/* ── Tool card ────────────────────────────────────────────── */}
        <div className="agd-tool-card">
          <span className="agd-tool-label">Tool</span>
          <code className="agd-tool-name">{request.toolName}</code>
          <span className="agd-tool-label agd-tool-label--inputs">Inputs</span>
          <pre className="agd-tool-inputs">{prettyInput}</pre>
        </div>

        {/* ── Rejection reason (shown when user clicks Reject) ─────── */}
        {rejecting && (
          <div className="agd-rejection">
            <label className="agd-rejection-label" htmlFor="agd-reason">
              Reason (optional)
            </label>
            <textarea
              id="agd-reason"
              className="agd-rejection-textarea"
              value={rejectionReason}
              onChange={e => setRejectionReason(e.target.value)}
              placeholder="Tell the agent why this tool call was rejected…"
              rows={3}
              disabled={busy}
              autoFocus
            />
          </div>
        )}

        {/* ── Error ────────────────────────────────────────────────── */}
        {error && <p className="agd-error">{error}</p>}

        {/* ── Actions ──────────────────────────────────────────────── */}
        <div className="agd-actions">
          {!rejecting && (
            <>
              <button
                className="btn btn--ghost btn--sm"
                onClick={onDismiss}
                disabled={busy}
              >
                Dismiss
              </button>
              <button
                className="btn btn--destructive btn--sm"
                onClick={() => setRejecting(true)}
                disabled={busy}
              >
                <X size={13} />
                Reject
              </button>
              <button
                className="btn btn--primary btn--sm"
                onClick={handleApprove}
                disabled={busy}
              >
                <Check size={13} />
                {busy ? "Approving…" : "Approve"}
              </button>
            </>
          )}
          {rejecting && (
            <>
              <button
                className="btn btn--ghost btn--sm"
                onClick={() => setRejecting(false)}
                disabled={busy}
              >
                Back
              </button>
              <button
                className="btn btn--destructive btn--sm"
                onClick={handleReject}
                disabled={busy}
              >
                <X size={13} />
                {busy ? "Rejecting…" : "Confirm rejection"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
