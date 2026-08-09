import assert from "node:assert/strict";
import test from "node:test";
import {
  defaultVadConfirmationMs,
  defaultVadNoiseMarginDb,
  defaultVadNoiseWindowMs,
  normalizeVadConfirmationMs,
  normalizeVadInitialSilenceTimeoutSecs,
  normalizeVadNoiseMargin,
  normalizeVadNoiseWindowMs,
} from "../src/domain/vadSettings.ts";

test("VAD defaults match the calibrated gate profile", () => {
  assert.equal(defaultVadNoiseMarginDb, 12);
  assert.equal(defaultVadConfirmationMs, 480);
  assert.equal(defaultVadNoiseWindowMs, 2000);
});

test("VAD numeric settings clamp and align to their slider steps", () => {
  assert.equal(normalizeVadNoiseMargin(99), 40);
  assert.equal(normalizeVadNoiseMargin(-5), 1);
  assert.equal(normalizeVadConfirmationMs(251), 260);
  assert.equal(normalizeVadConfirmationMs(5000), 2000);
  assert.equal(normalizeVadNoiseWindowMs(1249), 1200);
  assert.equal(normalizeVadNoiseWindowMs(1251), 1300);
  assert.equal(normalizeVadNoiseWindowMs(9999), 3000);
  assert.equal(normalizeVadInitialSilenceTimeoutSecs(2), 5);
  assert.equal(normalizeVadInitialSilenceTimeoutSecs(99), 60);
});
