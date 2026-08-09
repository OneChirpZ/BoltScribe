import assert from "node:assert/strict";
import test from "node:test";
import { translations } from "../src/domain/i18n.ts";
import { recordingOverlayLabel, recordingOverlayPhase, recordingOverlayTransition } from "../src/domain/overlay.ts";
import type { WorkflowStatus } from "../src/types.ts";

function status(mode: WorkflowStatus["mode"], revision: number): WorkflowStatus {
  const stage = mode === "starting" ? "starting" : mode === "recording" ? "recording" : mode === "error" ? "error" : "idle";
  return {
    mode,
    stage,
    message: mode,
    current_audio_path: null,
    last_record_id: null,
    revision,
  };
}

test("recording startup stages keep one capsule while using clear labels", () => {
  const starting = status("starting", 1);
  const recording = status("recording", 2);
  const waiting: WorkflowStatus = {
    ...recording,
    stage: "waiting_for_speech",
    message: "等待说话",
  };

  assert.equal(recordingOverlayPhase(starting), "phase-listening");
  assert.equal(recordingOverlayPhase(starting), recordingOverlayPhase(recording));
  assert.equal(recordingOverlayLabel(starting, translations["zh-CN"]), "正在启动");
  assert.equal(recordingOverlayLabel(waiting, translations["zh-CN"]), "等待说话");
  assert.equal(recordingOverlayLabel(recording, translations["zh-CN"]), "正在听取");
  assert.equal(recordingOverlayTransition(starting), "overlay-transition-starting");
  assert.equal(recordingOverlayTransition(waiting), "overlay-transition-waiting");
  assert.equal(recordingOverlayTransition(recording), "overlay-transition-active");
});

test("file ASR fallback remains a non-error processing capsule", () => {
  const fallback: WorkflowStatus = {
    mode: "processing",
    stage: "file_asr_fallback",
    message: "实时识别失败，正在使用录音文件重试",
    current_audio_path: null,
    last_record_id: null,
    revision: 3,
  };

  assert.equal(recordingOverlayPhase(fallback), "phase-processing");
  assert.equal(recordingOverlayLabel(fallback, translations["zh-CN"]), "回退文件识别");
  assert.equal(recordingOverlayLabel(fallback, translations["en-US"]), "Using File ASR");
});

test("only semantic error stage uses the red failure capsule", () => {
  const error = status("error", 4);
  error.message = "neutral text";

  assert.equal(recordingOverlayPhase(error), "phase-error");
  assert.equal(recordingOverlayLabel(error, translations["zh-CN"]), "处理失败");
});
