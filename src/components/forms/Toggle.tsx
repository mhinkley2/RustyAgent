// ─── Toggle / Switch ──────────────────────────────────────────────────────────

interface ToggleProps {
  id?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  /** Rendered below the label in text-muted. */
  description?: string;
  disabled?: boolean;
}

/**
 * Toggle — a switch control with an accessible label.
 * The entire row is clickable to activate the toggle.
 *
 * Layout:  label + optional description  [toggle pill]
 */
export function Toggle({
  id,
  checked,
  onChange,
  label,
  description,
  disabled = false,
}: ToggleProps) {
  return (
    <label
      className={[
        "form-toggle",
        checked ? "form-toggle--checked" : "",
        disabled ? "form-toggle--disabled" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="form-toggle__text">
        <span className="form-toggle__label">{label}</span>
        {description && (
          <span className="form-toggle__desc">{description}</span>
        )}
      </span>

      {/* Hidden native checkbox for accessibility */}
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        disabled={disabled}
        className="form-toggle__input"
        role="switch"
        aria-checked={checked}
      />

      <span className="form-toggle__track" aria-hidden="true">
        <span
          className={
            checked
              ? "form-toggle__thumb form-toggle__thumb--on"
              : "form-toggle__thumb"
          }
        />
      </span>
    </label>
  );
}

// ─── NumberInput ──────────────────────────────────────────────────────────────

interface NumberInputProps {
  id?: string;
  value: number | "";
  onChange: (value: number | "") => void;
  min?: number;
  max?: number;
  step?: number;
  /** When true, renders a "No limit" checkbox beside the input. */
  allowUnlimited?: boolean;
  unlimited?: boolean;
  onUnlimitedChange?: (unlimited: boolean) => void;
  hasError?: boolean;
  disabled?: boolean;
  placeholder?: string;
}

/**
 * NumberInput — number field with optional "No limit" checkbox.
 * Increment/decrement buttons appear on hover via CSS.
 */
export function NumberInput({
  id,
  value,
  onChange,
  min,
  max,
  step = 1,
  allowUnlimited = false,
  unlimited = false,
  onUnlimitedChange,
  hasError = false,
  disabled = false,
  placeholder,
}: NumberInputProps) {
  const isDisabled = disabled || unlimited;

  return (
    <div className="form-number-wrap">
      <input
        id={id}
        type="number"
        value={unlimited ? "" : value}
        onChange={(e) => {
          const raw = e.target.value;
          if (raw === "") {
            onChange("");
          } else {
            const n = Number(raw);
            if (!isNaN(n)) onChange(n);
          }
        }}
        min={min}
        max={max}
        step={step}
        disabled={isDisabled}
        placeholder={unlimited ? "No limit" : placeholder}
        aria-invalid={hasError}
        className={[
          "form-input form-number",
          hasError ? "form-input--error" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      />

      {allowUnlimited && onUnlimitedChange && (
        <label className="form-number__unlimited">
          <input
            type="checkbox"
            checked={unlimited}
            onChange={(e) => onUnlimitedChange(e.target.checked)}
            disabled={disabled}
          />
          <span>No limit</span>
        </label>
      )}
    </div>
  );
}
