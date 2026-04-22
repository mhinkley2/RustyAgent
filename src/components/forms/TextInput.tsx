// ─── TextInput ────────────────────────────────────────────────────────────────

interface TextInputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  hasError?: boolean;
}

/**
 * TextInput — themed <input type="text"> (or any text-like type).
 * Designed to be used as the `children` render of `<FormField>`.
 */
export function TextInput({ hasError, className, ...rest }: TextInputProps) {
  return (
    <input
      className={[
        "form-input",
        hasError ? "form-input--error" : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    />
  );
}

// ─── Textarea ─────────────────────────────────────────────────────────────────

interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  hasError?: boolean;
  /** Renders with monospace font (for system prompts, scripts). */
  mono?: boolean;
  /** When true, shows a character counter at the bottom-right. */
  maxLength?: number;
  currentLength?: number;
}

/**
 * Textarea — themed <textarea>. Vertically resizable, not horizontally.
 * Designed to be used as the `children` render of `<FormField>`.
 */
export function Textarea({
  hasError,
  mono,
  currentLength,
  maxLength,
  className,
  ...rest
}: TextareaProps) {
  return (
    <div className="form-textarea-wrap">
      <textarea
        className={[
          "form-textarea",
          hasError ? "form-textarea--error" : "",
          mono ? "form-textarea--mono" : "",
          className ?? "",
        ]
          .filter(Boolean)
          .join(" ")}
        maxLength={maxLength}
        {...rest}
      />
      {maxLength !== undefined && (
        <span className="form-textarea__counter" aria-live="polite">
          {currentLength ?? (rest.value as string | undefined)?.length ?? 0} /{" "}
          {maxLength}
        </span>
      )}
    </div>
  );
}
