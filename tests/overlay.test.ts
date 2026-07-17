import assert from "node:assert/strict";
import test from "node:test";
import { translations } from "../src/domain/i18n.ts";
import { recordingOverlayLabel, recordingOverlayPhase } from "../src/domain/overlay.ts";
import type { WorkflowStatus } from "../src/types.ts";

function status(mode: WorkflowStatus["mode"], revision: number): WorkflowStatus {
  return {
    mode,
    message: mode,
    current_audio_path: null,
    last_record_id: null,
    revision,
  };
}

test("starting and listening share one stable capsule presentation", () => {
  const starting = status("starting", 1);
  const recording = status("recording", 2);

  assert.equal(recordingOverlayPhase(starting), "phase-listening");
  assert.equal(recordingOverlayPhase(starting), recordingOverlayPhase(recording));
  assert.equal(
    recordingOverlayLabel(starting, translations["zh-CN"]),
    recordingOverlayLabel(recording, translations["zh-CN"]),
  );
  assert.equal(
    recordingOverlayLabel(starting, translations["en-US"]),
    recordingOverlayLabel(recording, translations["en-US"]),
  );
});
