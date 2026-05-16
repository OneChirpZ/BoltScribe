import type { ReactNode } from "react";

export default function PanelHeader({ title, action }: { title: string; action?: ReactNode }) {
  return (
    <div className="panel-header">
      <h1>{title}</h1>
      {action}
    </div>
  );
}
