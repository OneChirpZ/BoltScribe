export const audioLevelHistoryLength = 14;

export function createAudioLevelHistory() {
  return Array<number>(audioLevelHistoryLength).fill(0);
}

export function appendAudioLevelSample(history: number[], level: number) {
  const normalized = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
  const previous = history.slice(-(audioLevelHistoryLength - 1));
  const padding = Math.max(0, audioLevelHistoryLength - previous.length - 1);
  return [...Array<number>(padding).fill(0), ...previous, normalized];
}
