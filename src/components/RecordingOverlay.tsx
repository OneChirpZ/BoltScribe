import type { CSSProperties, PointerEvent } from "react";
import type { WorkflowStatus } from "../types";
import type { TextBundle } from "../domain/i18n";
import { maxRecordingOverlayScale, minRecordingOverlayScale, recordingOverlayLabel, recordingOverlayPhase } from "../domain/overlay";
import { clampNumber } from "../domain/numbers";

export default function RecordingOverlay({
  status,
  scale,
  onCancel,
  text,
}: {
  status: WorkflowStatus;
  scale: number;
  onCancel?: () => void;
  text: TextBundle;
}) {
  if (status.mode !== "recording" && status.mode !== "processing" && status.mode !== "error") {
    return null;
  }

  const phase = recordingOverlayPhase(status);
  const style = {
    "--recording-overlay-scale": String(clampNumber(scale, minRecordingOverlayScale, maxRecordingOverlayScale)),
  } as CSSProperties;

  return (
    <div
      className={`recording-overlay ${phase}`}
      role="status"
      aria-live="polite"
      style={style}
    >
      <span>{recordingOverlayLabel(status, text)}</span>
      <i aria-hidden="true" />
      <div className="voice-bars" aria-hidden="true">
        <b />
        <b />
        <b />
        <b />
        <b />
      </div>
      {onCancel && status.mode !== "error" ? (
        <button className="overlay-cancel" type="button" onPointerDown={(event) => event.stopPropagation()} onClick={onCancel} aria-label={text.overlay.cancel}>
          ×
        </button>
      ) : null}
    </div>
  );
}
