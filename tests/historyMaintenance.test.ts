import assert from "node:assert/strict";
import test from "node:test";
import { cleanupCutoffTimestamp, formatByteCount } from "../src/domain/historyMaintenance.ts";

test("cleanup cutoff handles day, week, and thirty-day month units", () => {
  const now = Date.parse("2026-07-31T12:00:00Z");
  assert.equal(cleanupCutoffTimestamp(now, 3, "day"), Date.parse("2026-07-28T12:00:00Z"));
  assert.equal(cleanupCutoffTimestamp(now, 2, "week"), Date.parse("2026-07-17T12:00:00Z"));
  assert.equal(cleanupCutoffTimestamp(now, 1, "month"), Date.parse("2026-07-01T12:00:00Z"));
});

test("cleanup age rejects empty, fractional, negative, and unsafe values", () => {
  const now = Date.parse("2026-07-31T12:00:00Z");
  assert.equal(cleanupCutoffTimestamp(now, 0, "day"), null);
  assert.equal(cleanupCutoffTimestamp(now, 1.5, "week"), null);
  assert.equal(cleanupCutoffTimestamp(now, -1, "month"), null);
  assert.equal(cleanupCutoffTimestamp(now, Number.MAX_SAFE_INTEGER, "month"), null);
  assert.equal(cleanupCutoffTimestamp(now, 36_501, "day"), null);
  assert.equal(cleanupCutoffTimestamp(now, 5_201, "week"), null);
  assert.equal(cleanupCutoffTimestamp(now, 1_201, "month"), null);
});

test("byte counts use compact binary units", () => {
  assert.equal(formatByteCount(0), "0 B");
  assert.equal(formatByteCount(512), "512 B");
  assert.equal(formatByteCount(1024), "1 KB");
  assert.equal(formatByteCount(1536), "1.5 KB");
  assert.equal(formatByteCount(12 * 1024 * 1024), "12 MB");
});
