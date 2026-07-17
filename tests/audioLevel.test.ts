import assert from "node:assert/strict";
import test from "node:test";
import { appendAudioLevelSample, audioLevelHistoryLength, createAudioLevelHistory } from "../src/domain/audioLevel.ts";

test("audio level history moves older samples left and appends the newest sample on the right", () => {
  let history = createAudioLevelHistory();
  history = appendAudioLevelSample(history, 0.25);
  history = appendAudioLevelSample(history, 0.75);

  assert.equal(history.length, audioLevelHistoryLength);
  assert.deepEqual(history.slice(-3), [0, 0.25, 0.75]);
});

test("audio level history clamps invalid and out-of-range samples", () => {
  let history: number[] = [];
  history = appendAudioLevelSample(history, -1);
  history = appendAudioLevelSample(history, Number.NaN);
  history = appendAudioLevelSample(history, 2);

  assert.deepEqual(history.slice(-3), [0, 0, 1]);
});
