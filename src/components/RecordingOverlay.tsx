import { useEffect, useState, type CSSProperties } from "react";
import type { WorkflowStatus } from "../types";
import type { TextBundle } from "../domain/i18n";
import { appendAudioLevelSample, createAudioLevelHistory } from "../domain/audioLevel";
import { maxRecordingOverlayScale, minRecordingOverlayScale, recordingOverlayLabel, recordingOverlayPhase } from "../domain/overlay";
import { clampNumber } from "../domain/numbers";

export default function RecordingOverlay({
  status,
  audioLevel,
  audioLevelSequence,
  baseWidth,
  scale,
  onCancel,
  text,
}: {
  status: WorkflowStatus;
  audioLevel: number;
  audioLevelSequence: number;
  baseWidth: number;
  scale: number;
  onCancel?: () => void;
  text: TextBundle;
}) {
  const [audioLevelHistory, setAudioLevelHistory] = useState(createAudioLevelHistory);

  useEffect(() => {
    if (status.mode === "recording") {
      setAudioLevelHistory((history) => appendAudioLevelSample(history, audioLevel));
    } else {
      setAudioLevelHistory(createAudioLevelHistory());
    }
  }, [audioLevel, audioLevelSequence, status.mode]);

  if (status.mode !== "starting" && status.mode !== "recording" && status.mode !== "processing" && status.mode !== "error") {
    return null;
  }

  const phase = recordingOverlayPhase(status);
  const label = recordingOverlayLabel(status, text);
  const transitionState = status.mode === "starting"
    ? "overlay-transition-starting"
    : status.mode === "recording"
      ? "overlay-transition-listening"
      : "overlay-transition-status";
  const canCancel = Boolean(onCancel) && status.mode !== "starting";
  const style = {
    "--recording-overlay-width": `${baseWidth}px`,
    "--recording-overlay-scale": String(clampNumber(scale, minRecordingOverlayScale, maxRecordingOverlayScale)),
  } as CSSProperties;

  return (
    <div
      className={`recording-overlay ${phase} ${transitionState}`}
      style={style}
    >
      <span className="overlay-label" role="status" aria-live="polite" aria-atomic="true">{label}</span>
      <i aria-hidden="true" />
      <div className="voice-visual" aria-hidden="true">
        <div className="startup-spinner" />
        <div className="voice-bars">
          {audioLevelHistory.map((level, index) => (
            <b
              key={index}
              style={{
                height: `${4 + level * 38}px`,
                opacity: 0.5 + level * 0.5,
              } as CSSProperties}
            />
          ))}
        </div>
      </div>
      {onCancel ? (
        <button
          className={`overlay-cancel${canCancel ? "" : " overlay-cancel-placeholder"}`}
          type="button"
          disabled={!canCancel}
          aria-hidden={!canCancel}
          tabIndex={canCancel ? 0 : -1}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={canCancel ? onCancel : undefined}
          aria-label={status.mode === "error" ? text.overlay.dismiss : text.overlay.cancel}
        >
          ×
        </button>
      ) : null}
    </div>
  );
}
