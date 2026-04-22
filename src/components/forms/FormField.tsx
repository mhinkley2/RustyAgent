import { useId } from "react";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface FormFieldProps {
  label: string;
  /** Use "div" when the child control renders its own <label> wrapper (e.g. Toggle). */
  labelAs?: "label" | "div";
  /** Marks the field as required with a subtle asterisk. */
  required?: boolean;
  /** Short helper text shown below the input. */
  helperText?: string;
  /** Validation error — shown below the field in error color. */
  error?: string;
  children: (id: string, hasError: boolean) => React.ReactNode;
  className?: string;
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * FormField — wrapper that provides label-above layout with helper text and
 * inline error messages.
 *
 * Usage:
 *   <FormField label="Agent Name" required error={errors.name}>
 *     {(id, hasError) => <TextInput id={id} hasError={hasError} {...} />}
 *   </FormField>
 */
export default function FormField({
  label,
  labelAs = "label",
  required,
  helperText,
  error,
  children,
  className,
}: FormFieldProps) {
  const id = useId();
  const errorId = `${id}-error`;
  const helperId = `${id}-helper`;
  const LabelTag = labelAs;

  return (
    <div
      className={[
        "form-field",
        error ? "form-field--error" : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <LabelTag
        className="form-field__label"
        {...(labelAs === "label" ? { htmlFor: id } : {})}
      >
        {label}
        {required && (
          <span className="form-field__required" aria-label="required">
            *
          </span>
        )}
      </LabelTag>

      {children(id, Boolean(error))}

      {helperText && !error && (
        <p id={helperId} className="form-field__helper">
          {helperText}
        </p>
      )}

      {error && (
        <p id={errorId} className="form-field__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
