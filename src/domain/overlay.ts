import type { WorkflowStatus } from "../types";
import type { TextBundle } from "./i18n";

export const defaultRecordingOverlayScale = 0.5;
export const minRecordingOverlayScale = 0.25;
export const maxRecordingOverlayScale = 1.0;

export function recordingOverlayLabel(status: WorkflowStatus, text: TextBundle) {
  const message = status.message || "";
  if (status.mode === "error" || message.includes("失败") || message.includes("错误")) {
    return text.overlay.failed;
  }
  if (status.mode === "recording") {
    return text.overlay.listening;
  }

  if (message.includes("粘贴完成")) {
    return text.overlay.pasteDone;
  }
  if (message.includes("纠错")) {
    return text.overlay.correcting;
  }
  if (message.includes("粘贴")) {
    return text.overlay.pasting;
  }
  if (message.includes("识别") || message.includes("转写") || message.includes("录音")) {
    return text.overlay.recognizing;
  }
  return text.overlay.processing;
}

export function recordingOverlayPhase(status: WorkflowStatus) {
  const message = status.message || "";
  if (status.mode === "error" || message.includes("失败") || message.includes("错误")) {
    return "phase-error";
  }
  if (status.mode === "recording") {
    return "phase-listening";
  }
  if (message.includes("粘贴")) {
    return "phase-pasted";
  }
  return "phase-processing";
}
