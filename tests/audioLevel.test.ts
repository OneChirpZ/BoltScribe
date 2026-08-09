import assert from "node:assert/strict";
import test from "node:test";
import { appendAudioLevelSample, audioLevelHistoryLength, createAudioLevelHistory, createAudioLevelHistoryState, dbfsToDisplayLevel, updateAudioLevelHistory } from "../src/domain/audioLevel.ts";

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

test("VAD dBFS levels use the same display response as the recording capsule", () => {
  assert.equal(dbfsToDisplayLevel(-96), 0);
  assert.equal(dbfsToDisplayLevel(-42.5), 0);
  assert.equal(dbfsToDisplayLevel(0), 1);
  assert.equal(dbfsToDisplayLevel(Number.NaN), 0);
  assert.ok(dbfsToDisplayLevel(-12) > dbfsToDisplayLevel(-24));
});

test("a stale sample cannot initialize a new recording meter", () => {
  let state = createAudioLevelHistoryState();
  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 4,
    sampleRevision: 3,
    sequence: 8,
    level: 1,
  });

  assert.deepEqual(state.samples, createAudioLevelHistory());

  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 4,
    sampleRevision: 4,
    sequence: 9,
    level: 0.5,
  });
  assert.equal(state.samples.at(-1), 0.5);
});

test("a recording-stage revision change keeps the waveform continuous", () => {
  let state = createAudioLevelHistoryState();
  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 4,
    sampleRevision: 4,
    sequence: 1,
    level: 0.35,
  });
  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 5,
    sampleRevision: 5,
    sequence: 2,
    level: 0.7,
  });

  assert.deepEqual(state.samples.slice(-2), [0.35, 0.7]);
  assert.equal(state.recordingRevision, 5);
});

test("late samples from an older recording cannot alter the current meter", () => {
  let state = createAudioLevelHistoryState();
  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 7,
    sampleRevision: 7,
    sequence: 2,
    level: 0,
  });
  const current = state;

  state = updateAudioLevelHistory(state, {
    recording: true,
    workflowRevision: 7,
    sampleRevision: 6,
    sequence: 3,
    level: 1,
  });

  assert.equal(state, current);
});
