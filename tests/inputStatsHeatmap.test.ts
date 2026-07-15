import assert from "node:assert/strict";
import test from "node:test";
import { buildInputHeatmapCells } from "../src/domain/inputStatsHeatmap.ts";

test("heatmap ends on today and fills the final seven-cell column with real dates", () => {
  const today = new Date(2026, 6, 12, 15, 30);
  const cells = buildInputHeatmapCells([], 8, today);

  assert.equal(cells.length, 56);
  assert.equal(cells.at(-1)?.date, "2026-07-12");
  assert.deepEqual(cells.slice(-7).map((cell) => cell.date), [
    "2026-07-06",
    "2026-07-07",
    "2026-07-08",
    "2026-07-09",
    "2026-07-10",
    "2026-07-11",
    "2026-07-12",
  ]);
});

test("heatmap ignores future records and fills missing past dates with zeroes", () => {
  const cells = buildInputHeatmapCells([
    { date: "2026-07-11", record_count: 3, character_count: 42, audio_duration_ms: 9000 },
    { date: "2026-07-13", record_count: 99, character_count: 999, audio_duration_ms: 9999 },
  ], 1, new Date(2026, 6, 12, 8));

  assert.equal(cells.length, 49);
  assert.equal(cells.at(-2)?.record_count, 3);
  assert.equal(cells.at(-1)?.date, "2026-07-12");
  assert.equal(cells.at(-1)?.record_count, 0);
  assert.equal(cells.some((cell) => cell.date === "2026-07-13"), false);
});
