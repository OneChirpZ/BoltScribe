import type { ReactNode } from "react";

export default function Field({ label, children, className = "" }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <label className={`field ${className}`.trim()}>
      <span className="field-label">{label}</span>
      {children}
    </label>
  );
}
