import type { DailyInputStats } from "../types";

const minHeatmapWeeks = 7;

export function buildInputHeatmapCells(daily: DailyInputStats[], weekCount: number, now = new Date()) {
  const byDate = new Map(daily.map((day) => [day.date, day]));
  const end = startOfLocalDay(now);
  const totalDays = Math.max(minHeatmapWeeks, weekCount) * 7;
  const start = new Date(end);
  start.setDate(end.getDate() - totalDays + 1);

  return Array.from({ length: totalDays }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    const key = formatDateKey(date);
    return byDate.get(key) ?? {
      date: key,
      record_count: 0,
      character_count: 0,
      audio_duration_ms: 0,
    };
  });
}

function startOfLocalDay(date: Date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function formatDateKey(date: Date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}
