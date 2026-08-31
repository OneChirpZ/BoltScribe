import { useEffect, useLayoutEffect, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";
import type { WorkflowStatus } from "../types";
import type { TextBundle } from "../domain/i18n";
import { createAudioLevelHistoryState, updateAudioLevelHistory } from "../domain/audioLevel";
import { maxRecordingOverlayScale, minRecordingOverlayScale, recordingOverlayEffectMargin, recordingOverlayLabel, recordingOverlayPhase, recordingOverlayTransition, shouldStartRecordingOverlayDrag } from "../domain/overlay";
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
  onDragStart,
  text,
}: {
  status: WorkflowStatus;
  audioLevel: number;
  audioLevelRevision: number;
  audioLevelSequence: number;
  baseWidth: number;
  scale: number;
  onCancel?: () => void;
  onDragStart?: () => void;
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
  const normalizedScale = clampNumber(scale, minRecordingOverlayScale, maxRecordingOverlayScale);
  const style = {
    "--recording-overlay-width": `${baseWidth}px`,
    "--recording-overlay-scale": String(normalizedScale),
    "--recording-overlay-effect-margin": `${recordingOverlayEffectMargin}px`,
    "--recording-overlay-edge-width": `${1 / normalizedScale}px`,
    "--recording-overlay-glow-blur": `${9 / normalizedScale}px`,
    "--recording-overlay-glow-spread": `${0.75 / normalizedScale}px`,
    "--recording-overlay-shadow-y": `${2 / normalizedScale}px`,
    "--recording-overlay-shadow-blur": `${8 / normalizedScale}px`,
    "--recording-overlay-inner-blur": `${18 / normalizedScale}px`,
  } as CSSProperties;

  function startDrag(event: ReactPointerEvent<HTMLDivElement>) {
    const target = event.target;
    const insideCancelControl = target instanceof Element && target.closest(".overlay-cancel") !== null;
    if (!shouldStartRecordingOverlayDrag(event.button, insideCancelControl)) {
      return;
    }
    event.preventDefault();
    onDragStart?.();
  }

  return (
    <div
      className={`recording-overlay ${phase} ${transitionState} overlay-stage-${status.stage}`}
      style={style}
      onPointerDown={startDrag}
    >
      <AnimatedOverlayLabel label={label} />
      <i aria-hidden="true" />
      <div className="voice-visual" aria-hidden="true">
        <div className="startup-spinner" />
        <AudioWaveform samples={audioLevelHistory.samples} />
        <div className="processing-indicator">
          <span />
          <span />
          <span />
        </div>
        <svg className="completion-mark" viewBox="0 0 42 42" focusable="false">
          <path d="M11 22.5 18.2 29.5 31.5 14.5" />
        </svg>
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

const overlayLabelTransitionMs = 240;

interface OverlayLabelVisualState {
  current: string;
  previous: string | null;
  transitionId: number;
}

function AnimatedOverlayLabel({ label }: { label: string }) {
  const [visual, setVisual] = useState<OverlayLabelVisualState>(() => ({
    current: label,
    previous: null,
    transitionId: 0,
  }));

  useLayoutEffect(() => {
    setVisual((current) => current.current === label ? current : {
      current: label,
      previous: current.current,
      transitionId: current.transitionId + 1,
    });
  }, [label]);

  useEffect(() => {
    if (visual.previous === null) {
      return;
    }
    const transitionId = visual.transitionId;
    const timer = window.setTimeout(() => {
      setVisual((current) => current.transitionId === transitionId
        ? { ...current, previous: null }
        : current);
    }, overlayLabelTransitionMs);
    return () => window.clearTimeout(timer);
  }, [visual.previous, visual.transitionId]);

  const changing = visual.previous !== null;
  return (
    <span
      className={`overlay-label${changing ? " overlay-label-changing" : ""}`}
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      {visual.previous !== null ? (
        <span
          key={`previous-${visual.transitionId}`}
          className="overlay-label-text overlay-label-previous"
          aria-hidden="true"
        >
          {visual.previous}
        </span>
      ) : null}
      <span
        key={`current-${visual.transitionId}`}
        className="overlay-label-text overlay-label-current"
      >
        {visual.current}
      </span>
    </span>
  );
}
