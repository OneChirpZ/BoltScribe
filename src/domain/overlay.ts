import type { AppConfig, RecordingOverlayPositionUpdate, WorkflowStatus } from "../types";
import type { AppLanguage, TextBundle } from "./i18n";

export const recordingOverlayBaseWidth = (language: AppLanguage) => language === "en-US" ? 400 : 340;

export const defaultRecordingOverlayScale = 0.5;
export const minRecordingOverlayScale = 0.25;
export const maxRecordingOverlayScale = 1.0;
export const defaultRecordingOverlaySnapRangePercent = 200;
export const minRecordingOverlaySnapRangePercent = 50;
export const maxRecordingOverlaySnapRangePercent = 500;

export function shouldStartRecordingOverlayDrag(button: number, insideCancelControl: boolean) {
  return button === 0 && !insideCancelControl;
}

export function mergeRecordingOverlayPosition(
  config: AppConfig,
  update: RecordingOverlayPositionUpdate,
): AppConfig {
  if (
    config.ui.recording_overlay_offset_x === update.recording_overlay_offset_x
    && config.ui.recording_overlay_offset_y === update.recording_overlay_offset_y
  ) {
    return config;
  }
  return {
    ...config,
    ui: {
      ...config.ui,
      recording_overlay_offset_x: update.recording_overlay_offset_x,
      recording_overlay_offset_y: update.recording_overlay_offset_y,
    },
  };
}

export function recordingOverlayLabel(status: WorkflowStatus, text: TextBundle) {
  switch (status.stage) {
    case "error":
      return text.overlay.failed;
    case "starting":
      return text.overlay.starting;
    case "waiting_for_speech":
      return text.overlay.waitingForSpeech;
    case "recording":
      return text.overlay.listening;
    case "file_asr_fallback":
      return text.overlay.fileAsrFallback;
    case "correcting":
      return text.overlay.correcting;
    case "pasting":
      return text.overlay.pasting;
    case "complete":
      return text.overlay.pasteDone;
    case "recognizing":
      return text.overlay.recognizing;
    default:
      return text.overlay.processing;
  }
}

export function recordingOverlayTransition(status: WorkflowStatus) {
  switch (status.stage) {
    case "starting":
      return "overlay-transition-starting";
    case "waiting_for_speech":
      return "overlay-transition-waiting";
    case "recording":
      return "overlay-transition-active";
    default:
      return "overlay-transition-status";
  }
}

export function recordingOverlayPhase(status: WorkflowStatus) {
  switch (status.stage) {
    case "error":
      return "phase-error";
    case "starting":
    case "waiting_for_speech":
    case "recording":
      return "phase-listening";
    case "complete":
      return "phase-pasted";
    default:
      return "phase-processing";
  }
}
