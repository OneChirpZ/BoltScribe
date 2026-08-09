import { useEffect, useState, type CSSProperties } from "react";
import type { WorkflowStatus } from "../types";
import type { TextBundle } from "../domain/i18n";
import { createAudioLevelHistoryState, updateAudioLevelHistory } from "../domain/audioLevel";
import { maxRecordingOverlayScale, minRecordingOverlayScale, recordingOverlayLabel, recordingOverlayPhase, recordingOverlayTransition } from "../domain/overlay";
import { clampNumber } from "../domain/numbers";
import AudioWaveform from "./AudioWaveform";

export default function RecordingOverlay({
  status,
  audioLevel,
  audioLevelRevision,
  audioLevelSequence,
  baseWidth,
  scale,
  onCancel,
  text,
}: {
  status: WorkflowStatus;
  audioLevel: number;
  audioLevelRevision: number;
  audioLevelSequence: number;
  baseWidth: number;
  scale: number;
  onCancel?: () => void;
  text: TextBundle;
}) {
  const [audioLevelHistory, setAudioLevelHistory] = useState(() => createAudioLevelHistoryState(audioLevelSequence));

  useEffect(() => {
    setAudioLevelHistory((history) => updateAudioLevelHistory(history, {
      recording: status.mode === "recording",
      workflowRevision: status.revision,
      sampleRevision: audioLevelRevision,
      sequence: audioLevelSequence,
      level: audioLevel,
    }));
  }, [audioLevel, audioLevelRevision, audioLevelSequence, status.mode, status.revision]);

  if (status.mode !== "starting" && status.mode !== "recording" && status.mode !== "processing" && status.mode !== "error") {
    return null;
  }

  const phase = recordingOverlayPhase(status);
  const label = recordingOverlayLabel(status, text);
  const transitionState = recordingOverlayTransition(status);
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
      <span className="overlay-label" role="status" aria-live="polite" aria-atomic="true">
        <span key={`${status.stage}-${label}`} className="overlay-label-text">{label}</span>
      </span>
      <i aria-hidden="true" />
      <div className="voice-visual" aria-hidden="true">
        <div className="startup-spinner" />
        <AudioWaveform samples={audioLevelHistory.samples} />
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
