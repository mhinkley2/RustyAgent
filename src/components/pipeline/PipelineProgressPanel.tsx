import { CheckCircle, Circle, Loader, XCircle, GitBranch } from "lucide-react";
import type { PipelineProgress, StepProgress, StepStatus } from "../../types/board";
import "./PipelineProgressPanel.css";

interface PipelineProgressPanelProps {
  progress: PipelineProgress;
}

function stepIcon(status: StepStatus) {
  switch (status) {
    case "running":
      return <Loader size={16} className="pipeline-step__icon pipeline-step__icon--running" />;
    case "done":
      return <CheckCircle size={16} className="pipeline-step__icon pipeline-step__icon--done" />;
    case "failed":
      return <XCircle size={16} className="pipeline-step__icon pipeline-step__icon--failed" />;
    default:
      return <Circle size={16} className="pipeline-step__icon pipeline-step__icon--pending" />;
  }
}

function StepRow({ step, isLast, mode }: { step: StepProgress; isLast: boolean; mode: string }) {
  return (
    <div className="pipeline-step">
      <div className="pipeline-step__track">
        <div className={`pipeline-step__dot pipeline-step__dot--${step.status}`}>
          {stepIcon(step.status)}
        </div>
        {!isLast && mode === "sequential" && (
          <div className={`pipeline-step__connector ${step.status === "done" ? "pipeline-step__connector--done" : ""}`} />
        )}
      </div>
      <div className="pipeline-step__body">
        <div className="pipeline-step__label">
          <span className="pipeline-step__index">Step {step.index + 1}</span>
          <span className="pipeline-step__name">{step.label}</span>
        </div>
        <div className={`pipeline-step__badge pipeline-step__badge--${step.status}`}>
          {step.status}
        </div>
        {step.runId && (
          <span className="pipeline-step__run-id" title={step.runId}>
            run: {step.runId.slice(0, 8)}…
          </span>
        )}
      </div>
    </div>
  );
}

export function PipelineProgressPanel({ progress }: PipelineProgressPanelProps) {
  const doneCount = progress.steps.filter((s) => s.status === "done").length;
  const totalCount = progress.steps.length;

  return (
    <div className="pipeline-panel">
      <div className="pipeline-panel__header">
        <GitBranch size={16} className="pipeline-panel__icon" />
        <span className="pipeline-panel__title">Pipeline</span>
        <span className="pipeline-panel__mode">{progress.mode}</span>
        <span className={`pipeline-panel__status pipeline-panel__status--${progress.status}`}>
          {progress.status}
        </span>
        <span className="pipeline-panel__progress">
          {doneCount}/{totalCount}
        </span>
      </div>

      {progress.status === "running" && (
        <div className="pipeline-panel__progress-bar">
          <div
            className="pipeline-panel__progress-fill"
            style={{ width: `${totalCount > 0 ? (doneCount / totalCount) * 100 : 0}%` }}
          />
        </div>
      )}

      <div className="pipeline-panel__steps">
        {progress.steps.map((step, i) => (
          <StepRow
            key={step.index}
            step={step}
            isLast={i === progress.steps.length - 1}
            mode={progress.mode}
          />
        ))}
      </div>
    </div>
  );
}
