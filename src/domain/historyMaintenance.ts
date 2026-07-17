import type { RecordingCleanupUnit } from "../types";

const maxCleanupAmounts: Record<RecordingCleanupUnit, number> = {
  day: 36_500,
  week: 5_200,
  month: 1_200,
};

export function cleanupCutoffTimestamp(now: number, amount: number, unit: RecordingCleanupUnit) {
  if (!Number.isSafeInteger(amount) || amount < 1 || amount > maxCleanupAmounts[unit]) {
    return null;
  }
  const days = amount * (unit === "week" ? 7 : unit === "month" ? 30 : 1);
  if (!Number.isSafeInteger(days)) {
    return null;
  }
  const cutoff = now - days * 24 * 60 * 60 * 1000;
  return Number.isFinite(new Date(cutoff).getTime()) ? cutoff : null;
}

export function formatByteCount(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** unitIndex;
  const digits = unitIndex === 0 || value >= 10 || Number.isInteger(value) ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}
