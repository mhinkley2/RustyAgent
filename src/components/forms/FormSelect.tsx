import { ChevronDown, X, Loader2 } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface SelectOption<T extends string = string> {
  value: T;
  label: string;
  disabled?: boolean;
}

interface FormSelectProps<T extends string = string> {
  id?: string;
  value: T | "";
  options: SelectOption<T>[];
  onChange: (value: T | "") => void;
  placeholder?: string;
  /** Shows a spinner inside the select — used while async options load. */
  loading?: boolean;
  /** Renders a clear (×) button when the field is optional. */
  clearable?: boolean;
  disabled?: boolean;
  hasError?: boolean;
  className?: string;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * FormSelect — themed select box. Uses native <select> styled to match the
 * design system; custom chrome via CSS appearance:none + chevron overlay.
 */
export function FormSelect<T extends string = string>({
  id,
  value,
  options,
  onChange,
  placeholder = "Select…",
  loading = false,
  clearable = false,
  disabled = false,
  hasError = false,
  className,
}: FormSelectProps<T>) {
  return (
    <div
      className={[
        "form-select-wrap",
        hasError ? "form-select-wrap--error" : "",
        disabled ? "form-select-wrap--disabled" : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value as T | "")}
        disabled={disabled || loading}
        className="form-select"
        aria-invalid={hasError}
      >
        <option value="" disabled hidden>
          {loading ? "Loading…" : placeholder}
        </option>
        {options.map((opt) => (
          <option key={opt.value} value={opt.value} disabled={opt.disabled}>
            {opt.label}
          </option>
        ))}
      </select>

      {/* Right-side adornment: spinner or chevron + optional clear button */}
      <div className="form-select__adornment" aria-hidden="true">
        {loading ? (
          <Loader2 size={14} className="form-select__spinner" />
        ) : (
          <ChevronDown size={14} />
        )}
      </div>

      {clearable && value && !disabled && (
        <button
          type="button"
          className="form-select__clear"
          onClick={() => onChange("")}
          aria-label="Clear selection"
          tabIndex={-1}
        >
          <X size={12} />
        </button>
      )}
    </div>
  );
}
