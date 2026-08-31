import { ChevronDown } from "lucide-react";

import type { AgentProfile } from "../../types/agent";
import { UNASSIGNED, pickerOptions } from "./assignment";

interface AgentPickerProps {
  agents: AgentProfile[];
  /** The currently assigned profile id, or null for unassigned. */
  value: string | null;
  /** `null` means the user chose Unassigned. */
  onChange: (agentId: string | null) => void;
  /** Required: several of these render with no visible label beside them. */
  ariaLabel: string;
  disabled?: boolean;
  autoFocus?: boolean;
  onBlur?: () => void;
  onKeyDown?: (e: React.KeyboardEvent<HTMLSelectElement>) => void;
  /**
   * Applied to the wrapper, not the select.
   *
   * A picker on a card sits inside a drag handle that also opens the detail
   * panel, so the card needs to stop those events before they leave the
   * control — including the ones that land on the chevron rather than the
   * select itself.
   */
  onClick?: (e: React.MouseEvent<HTMLDivElement>) => void;
  onPointerDown?: (e: React.PointerEvent<HTMLDivElement>) => void;
  /** What the empty option says. "Unassigned" on a story, "Its agent" for a run. */
  unassignedLabel?: string;
  className?: string;
}

/**
 * Choose an agent profile, or none.
 *
 * A native `<select>` rather than a popover: this renders inside a card that is
 * also a drag handle and inside a slide-out panel, and a hand-rolled menu would
 * have to solve focus, positioning and escape in both. The browser already has.
 *
 * `FormSelect` is the app's usual select but does not fit here — its empty
 * option is `disabled hidden`, so a user could never choose Unassigned again,
 * and it takes no accessible name, which every one of these needs.
 */
export function AgentPicker({
  agents,
  value,
  onChange,
  ariaLabel,
  disabled,
  autoFocus,
  onBlur,
  onKeyDown,
  onClick,
  onPointerDown,
  unassignedLabel = "Unassigned",
  className,
}: AgentPickerProps) {
  const options = pickerOptions(agents, value);

  return (
    <div
      className={["form-select-wrap", "agent-picker", className ?? ""].filter(Boolean).join(" ")}
      onClick={onClick}
      onPointerDown={onPointerDown}
    >
      <select
        className="form-select agent-picker__select"
        value={value ?? UNASSIGNED}
        aria-label={ariaLabel}
        disabled={disabled}
        autoFocus={autoFocus}
        onBlur={onBlur}
        onKeyDown={onKeyDown}
        onChange={(e) => onChange(e.target.value === UNASSIGNED ? null : e.target.value)}
      >
        <option value={UNASSIGNED}>{unassignedLabel}</option>
        {options.map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </select>
      <div className="form-select__adornment" aria-hidden="true">
        <ChevronDown size={14} />
      </div>
    </div>
  );
}
