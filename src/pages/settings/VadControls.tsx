import { useEffect, useRef, useState } from "react";
import HelpTip from "../../components/HelpTip";
import { normalizeSteppedInt } from "../../domain/vadSettings";

export function VadRangeNumberControl({
  label,
  value,
  min,
  max,
  step,
  unit,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const editingRef = useRef(false);

  useEffect(() => {
    if (!editingRef.current) {
      setDraft(String(value));
    }
  }, [value]);

  function commitDraft(raw: string) {
    const parsed = Number(raw);
    const next = raw.trim() && Number.isFinite(parsed)
      ? normalizeSteppedInt(parsed, min, max, step)
      : value;
    editingRef.current = false;
    setDraft(String(next));
    if (next !== value) {
      onChange(next);
    }
  }

  return (
    <div className="range-with-value vad-range-control">
      <input
        className="range-input"
        type="range"
        aria-label={`${label} slider`}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => {
          const next = normalizeSteppedInt(Number(event.target.value), min, max, step);
          setDraft(String(next));
          onChange(next);
        }}
      />
      <input
        type="number"
        aria-label={`${label} value`}
        inputMode="numeric"
        min={min}
        max={max}
        step={step}
        value={draft}
        onFocus={() => {
          editingRef.current = true;
        }}
        onChange={(event) => {
          const raw = event.target.value;
          setDraft(raw);
          const parsed = Number(raw);
          if (raw.trim() && Number.isFinite(parsed) && parsed >= min && parsed <= max) {
            const next = normalizeSteppedInt(parsed, min, max, step);
            setDraft(String(next));
            onChange(next);
          }
        }}
        onBlur={(event) => commitDraft(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.currentTarget.blur();
          }
        }}
      />
      <span>{unit}</span>
    </div>
  );
}

export function VadFieldLabel({ label, help }: { label: string; help: string }) {
  return (
    <span className="vad-field-label">
      <span>{label}</span>
      <HelpTip content={help} />
    </span>
  );
}

export function formatVadDbfs(value: number): string {
  return Number.isFinite(value) ? `${Math.round(value)} dB` : "—";
}

export function clampUnitInterval(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return Math.min(1, Math.max(0, value));
}
