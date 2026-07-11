import { useId, type ReactNode } from "react";

export default function Field({
  label,
  children,
  className = "",
  group = false,
}: {
  label: ReactNode;
  children: ReactNode;
  className?: string;
  group?: boolean;
}) {
  const labelId = useId();
  if (group) {
    return (
      <div className={`field ${className}`.trim()} role="group" aria-labelledby={labelId}>
        <span className="field-label" id={labelId}>{label}</span>
        {children}
      </div>
    );
  }

  return (
    <label className={`field ${className}`.trim()}>
      <span className="field-label">{label}</span>
      {children}
    </label>
  );
}
