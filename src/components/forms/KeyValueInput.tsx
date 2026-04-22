import { Plus, X } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface KeyValuePair {
  id: string;   /* stable identity for React keys */
  key: string;
  value: string;
}

interface KeyValueInputProps {
  pairs: KeyValuePair[];
  onChange: (pairs: KeyValuePair[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  addLabel?: string;
  disabled?: boolean;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeId() {
  return Math.random().toString(36).slice(2, 9);
}

// ─── Component ────────────────────────────────────────────────────────────────

/**
 * KeyValueInput — dynamic list of key/value pairs (env vars, args).
 *
 * Layout:
 *   ┌─────────────┬─────────────┬───┐
 *   │ Key         │ Value       │ × │
 *   └─────────────┴─────────────┴───┘
 *   [+ Add variable]
 */
export function KeyValueInput({
  pairs,
  onChange,
  keyPlaceholder = "Key",
  valuePlaceholder = "Value",
  addLabel = "Add variable",
  disabled = false,
}: KeyValueInputProps) {
  function updatePair(id: string, field: "key" | "value", val: string) {
    onChange(pairs.map((p) => (p.id === id ? { ...p, [field]: val } : p)));
  }

  function removePair(id: string) {
    onChange(pairs.filter((p) => p.id !== id));
  }

  function addPair() {
    onChange([...pairs, { id: makeId(), key: "", value: "" }]);
  }

  return (
    <div className="kv-input">
      {pairs.length > 0 && (
        <ul className="kv-input__list">
          {pairs.map((pair) => (
            <li key={pair.id} className="kv-input__row">
              <input
                type="text"
                value={pair.key}
                onChange={(e) => updatePair(pair.id, "key", e.target.value)}
                placeholder={keyPlaceholder}
                disabled={disabled}
                className="form-input kv-input__key"
                aria-label="Key"
                spellCheck={false}
              />
              <input
                type="text"
                value={pair.value}
                onChange={(e) => updatePair(pair.id, "value", e.target.value)}
                placeholder={valuePlaceholder}
                disabled={disabled}
                className="form-input kv-input__value"
                aria-label="Value"
                spellCheck={false}
              />
              <button
                type="button"
                className="kv-input__remove"
                onClick={() => removePair(pair.id)}
                disabled={disabled}
                aria-label="Remove row"
                tabIndex={-1}
              >
                <X size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}

      <button
        type="button"
        className="kv-input__add"
        onClick={addPair}
        disabled={disabled}
      >
        <Plus size={13} aria-hidden="true" />
        {addLabel}
      </button>
    </div>
  );
}
