export const audioLevelHistoryLength = 14;
const audioLevelFloorDbfs = -42.5;
const audioLevelCeilingDbfs = 0;
const audioLevelResponseExponent = 0.7;

export interface AudioLevelHistoryState {
  samples: number[];
  recordingRevision: number | null;
  lastSequence: number;
}

export interface AudioLevelFrame {
  recording: boolean;
  workflowRevision: number;
  sampleRevision: number;
  sequence: number;
  level: number;
}

export function createAudioLevelHistory() {
  return Array<number>(audioLevelHistoryLength).fill(0);
}

export function appendAudioLevelSample(history: number[], level: number) {
  const normalized = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
  const previous = history.slice(-(audioLevelHistoryLength - 1));
  const padding = Math.max(0, audioLevelHistoryLength - previous.length - 1);
  return [...Array<number>(padding).fill(0), ...previous, normalized];
}

export function dbfsToDisplayLevel(dbfs: number) {
  if (!Number.isFinite(dbfs)) {
    return 0;
  }
  const normalized = Math.min(1, Math.max(0, (dbfs - audioLevelFloorDbfs) / (audioLevelCeilingDbfs - audioLevelFloorDbfs)));
  return normalized ** audioLevelResponseExponent;
}

export function createAudioLevelHistoryState(lastSequence = 0): AudioLevelHistoryState {
  return {
    samples: createAudioLevelHistory(),
    recordingRevision: null,
    lastSequence,
  };
}

export function updateAudioLevelHistory(state: AudioLevelHistoryState, frame: AudioLevelFrame): AudioLevelHistoryState {
  if (!frame.recording) {
    return createAudioLevelHistoryState(frame.sequence);
  }
  if (frame.sampleRevision !== frame.workflowRevision || frame.sequence <= state.lastSequence) {
    return state;
  }
  if (state.recordingRevision !== frame.workflowRevision) {
    const history = state.recordingRevision === null ? createAudioLevelHistory() : state.samples;
    return {
      samples: appendAudioLevelSample(history, frame.level),
      recordingRevision: frame.workflowRevision,
      lastSequence: frame.sequence,
    };
  }
  return {
    samples: appendAudioLevelSample(state.samples, frame.level),
    recordingRevision: state.recordingRevision,
    lastSequence: frame.sequence,
  };
}
