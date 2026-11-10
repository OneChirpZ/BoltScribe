import { providerPresets } from "../../domain/providers";
import { cleanupCutoffTimestamp } from "../../domain/historyMaintenance";
import {
  defaultVadConfirmationMs,
  defaultVadNoiseMarginDb,
  defaultVadNoiseWindowMs,
  normalizeVadConfirmationMs,
  normalizeVadNoiseMargin,
  normalizeVadNoiseWindowMs,
} from "../../domain/vadSettings";
import type { AppConfig, AudioLevelSample, HistoryRecord, RecordingCleanupPreview, RecordingCleanupUnit, WorkflowStatusCode } from "../../types";
import type { AppApi } from "./contracts";
import { clonePreview, createBrowserPreviewState, type BrowserPreviewState } from "./previewData";
import { defaultLearningConfig, emptyLearningStatus } from "../../domain/learning";

interface BrowserPreviewAppApiOptions {
  appVersion: string;
  locationHref?: string;
}

export function createBrowserPreviewAppApi({ appVersion, locationHref }: BrowserPreviewAppApiOptions): AppApi {
  const state = createBrowserPreviewState(locationHref);
  const learningListeners = new Set<() => void>();

  return {
    learning: {
      loadActivity: (limit, offset = 0, recordId) => {
        const matching = state.learningActivity
          .filter((item) => !recordId || item.record_id === recordId)
          .map(({ edited_text: _editedText, model_output: _modelOutput, ...item }) => item);
        const active = matching.filter((item) => item.finished_at === null);
        const history = matching.filter((item) => item.finished_at !== null);
        const pageSize = Math.max(1, Math.min(100, limit));
        return Promise.resolve(clonePreview({ active, history: history.slice(offset, offset + pageSize), has_more: history.length > offset + pageSize }));
      },
      loadSource: (recordId, activityId) => {
        const matching = state.learningActivity.filter((item) => item.record_id === recordId);
        const source = activityId ? matching.find((item) => item.id === activityId)
          : matching.find((item) => item.state === "learned") ?? matching[0];
        return Promise.resolve(clonePreview(source ?? null));
      },
      loadEntries: () => Promise.resolve(clonePreview(state.learnedEntries)),
      deleteEntry: (id) => {
        state.learnedEntries = state.learnedEntries.filter((entry) => entry.id !== id);
        learningListeners.forEach((handler) => handler());
        return Promise.resolve(clonePreview(state.learnedEntries));
      },
      setEntryEnabled: (id, enabled) => {
        state.learnedEntries = state.learnedEntries.map((entry) => entry.id === id ? { ...entry, enabled } : entry);
        learningListeners.forEach((handler) => handler());
        return Promise.resolve(clonePreview(state.learnedEntries));
      },
      getStatus: () => {
        const status = emptyLearningStatus();
        status.observation.active = state.learningActivity.some((item) => item.state === "observing");
        status.processing = state.learningActivity.some((item) => item.state === "analyzing");
        status.pending = state.learningActivity.some((item) => item.state === "queued");
        return Promise.resolve(status);
      },
      listenUpdated: (handler) => {
        learningListeners.add(handler);
        return Promise.resolve(() => { learningListeners.delete(handler); });
      },
      listenOpenRequested: () => Promise.resolve(() => undefined),
      takeOpenRequest: () => Promise.resolve(false),
    },
    config: {
      listServiceModels: async (id) => {
        const service = state.config.model_services.find((item) => item.id === id);
        if (!service) throw new Error("Unknown model service");
        return [...(providerPresets[service.provider as keyof typeof providerPresets]?.models ?? [])];
      },
      loadConfig: () => Promise.resolve(clonePreview(state.config)),
      saveConfig: (config) => {
        state.config = clonePreview(config);
        return Promise.resolve({ config: clonePreview(state.config), warnings: [] });
      },
      exportConfig: (config) => Promise.resolve(`browser-preview-config-${config.ui.app_language}.json`),
      importConfig: (raw) => {
        state.config = clonePreview(JSON.parse(raw) as AppConfig);
        state.config.learning ??= defaultLearningConfig();
        return Promise.resolve({
          config: clonePreview(state.config),
          report: {
            format: "browser-preview",
            version: null,
            missing_fields: [],
            unknown_fields: [],
            invalid_fields: [],
            notes: [],
          },
          warnings: [],
        });
      },
      listenConfigUpdated: () => Promise.resolve(() => undefined),
      listenConfigCloseRequested: () => Promise.resolve(() => undefined),
    },
    overlay: {
      startRecordingOverlayDrag: () => Promise.resolve(),
      listenRecordingOverlayPositionUpdated: () => Promise.resolve(() => undefined),
    },
    workflow: {
      getStatus: () => Promise.resolve(clonePreview(state.status)),
      toggleRecording: () => {
        const revision = state.status.revision + 1;
        state.status = state.status.mode === "recording"
          ? idleStatus("就绪", revision, "ready")
          : {
              mode: "recording",
              stage: "recording",
              code: "recording",
              detail: null,
              message: "正在录音，再次按快捷键停止",
              current_audio_path: null,
              last_record_id: null,
              revision,
            };
        return Promise.resolve(clonePreview(state.status));
      },
      cancelCurrentWorkflow: () => {
        state.status = idleStatus("已取消本次转写", state.status.revision + 1, "cancelled");
        return Promise.resolve(clonePreview(state.status));
      },
      listenWorkflowStatus: (handler) => {
        handler(clonePreview(state.status));
        return Promise.resolve(() => undefined);
      },
    },
    history: {
      loadRecord: (id) => Promise.resolve(clonePreview(state.history.find((record) => record.id === id) ?? null)),
      loadHistory: (limit, offset = 0) => Promise.resolve(clonePreview(state.history.slice(offset, offset + limit))),
      deleteHistoryRecord: (id) => {
        const index = state.history.findIndex((record) => record.id === id);
        if (index < 0) {
          return Promise.resolve({ deleted_records: 0, deleted_audio_files: 0, freed_bytes: 0 });
        }
        const [record] = state.history.splice(index, 1);
        const deletedAudioFiles = record.audio_path ? 1 : 0;
        return Promise.resolve({
          deleted_records: 1,
          deleted_audio_files: deletedAudioFiles,
          freed_bytes: deletedAudioFiles * state.recordingBytes,
        });
      },
      retryHistoryRecord: (id) => retryHistoryRecord(state, id),
      cleanupRecordingFiles: (amount, unit) => cleanupRecordingFiles(state, amount, unit),
      previewRecordingCleanup: (amount, unit) => {
        try {
          const cutoff = cleanupCutoff(amount, unit);
          const { eligiblePaths: _eligiblePaths, ...preview } = browserRecordingCleanupPreview(state, cutoff);
          return Promise.resolve(preview);
        } catch (error) {
          return Promise.reject(error);
        }
      },
      loadStats: () => Promise.resolve(clonePreview(state.stats)),
      listenHistoryUpdated: () => Promise.resolve(() => undefined),
    },
    audio: {
      loadAudioInputDevices: () => Promise.resolve(clonePreview(state.audioInputDevices)),
      loadAudioOutputDevices: () => Promise.resolve(clonePreview(state.audioOutputDevices)),
      restartSystemAudioService: () => Promise.resolve(true),
      startVadTest: (audioConfig) => {
        const noiseMarginDb = normalizeVadNoiseMargin(
          audioConfig.voice_activity_detection.noise_margin_db ?? defaultVadNoiseMarginDb,
        );
        const confirmationMs = normalizeVadConfirmationMs(
          audioConfig.voice_activity_detection.confirmation_ms ?? defaultVadConfirmationMs,
        );
        const noiseWindowMs = normalizeVadNoiseWindowMs(
          audioConfig.voice_activity_detection.noise_window_ms ?? defaultVadNoiseWindowMs,
        );
        state.vadTestStatus = {
          ...state.vadTestStatus,
          mode: "listening",
          raw_voice_active: false,
          voice_active: false,
          noise_calibrated: true,
          noise_floor: -52,
          trigger_threshold: Math.min(-1, -52 + noiseMarginDb),
          trigger_progress: 0,
          noise_margin_db: noiseMarginDb,
          confirmation_ms: confirmationMs,
          noise_window_ms: noiseWindowMs,
          revision: state.vadTestStatus.revision + 1,
        };
        return Promise.resolve(clonePreview(state.vadTestStatus));
      },
      updateVadTestSettings: (noiseMarginDb, confirmationMs, noiseWindowMs) => {
        const normalizedMargin = normalizeVadNoiseMargin(noiseMarginDb);
        state.vadTestStatus = {
          ...state.vadTestStatus,
          noise_margin_db: normalizedMargin,
          confirmation_ms: normalizeVadConfirmationMs(confirmationMs),
          noise_window_ms: normalizeVadNoiseWindowMs(noiseWindowMs),
          trigger_threshold: Math.min(-1, state.vadTestStatus.noise_floor + normalizedMargin),
          trigger_progress: 0,
          revision: state.vadTestStatus.revision + 1,
        };
        return Promise.resolve(clonePreview(state.vadTestStatus));
      },
      stopVadTest: () => {
        state.vadTestStatus = {
          ...state.vadTestStatus,
          mode: "idle",
          raw_voice_active: false,
          voice_active: false,
          trigger_progress: 0,
          revision: state.vadTestStatus.revision + 1,
        };
        return Promise.resolve(clonePreview(state.vadTestStatus));
      },
      getVadTestStatus: () => Promise.resolve(clonePreview(state.vadTestStatus)),
      listenVadTestStatus: () => Promise.resolve(() => undefined),
      listenAudioLevel: (handler) => listenAudioLevel(state, handler),
    },
    permissions: {
      accessibilityPermissionGranted: () => Promise.resolve(true),
      requestAccessibilityPermission: () => Promise.resolve(true),
      openAccessibilitySettings: () => Promise.resolve(),
      inputMonitoringPermissionGranted: () => Promise.resolve(true),
      requestInputMonitoringPermission: () => Promise.resolve(true),
      openInputMonitoringSettings: () => Promise.resolve(),
      requestMicrophonePermission: () => Promise.resolve(true),
    },
    system: {
      getAppVersion: () => Promise.resolve(appVersion),
      openAppDir: () => Promise.resolve(),
      openGitHubRepository: () => {
        window.open("https://github.com/OneChirpZ/BoltScribe", "_blank", "noopener,noreferrer");
        return Promise.resolve();
      },
      getDataDir: () => Promise.resolve(clonePreview(state.dataDirInfo)),
      chooseDataDir: () => Promise.resolve("D:\\BoltScribeData"),
      setDataDir: (path) => {
        state.dataDirInfo = {
          path,
          default_path: state.dataDirInfo.default_path,
          is_default: path === state.dataDirInfo.default_path,
          cleanup_warning: null,
        };
        return Promise.resolve(clonePreview(state.dataDirInfo));
      },
      resetDataDir: () => {
        state.dataDirInfo = clonePreview(state.defaultDataDirInfo);
        return Promise.resolve(clonePreview(state.dataDirInfo));
      },
      closeMainWindow: () => Promise.resolve(),
      applyFnTrigger: () => Promise.resolve(),
      copyTextToClipboard: (text) => navigator.clipboard?.writeText(text) ?? Promise.resolve(),
    },
  } satisfies AppApi;
}

function idleStatus(message: string, revision: number, code: WorkflowStatusCode) {
  return {
    mode: "idle" as const,
    stage: "idle" as const,
    code,
    detail: null,
    message,
    current_audio_path: null,
    last_record_id: null,
    revision,
  };
}

function cleanupCutoff(amount: number, unit: RecordingCleanupUnit) {
  const cutoff = cleanupCutoffTimestamp(Date.now(), amount, unit);
  if (cutoff === null) {
    throw new Error("Recording cleanup amount must be a positive integer");
  }
  return cutoff;
}

function cleanupRecordingFiles(state: BrowserPreviewState, amount: number, unit: RecordingCleanupUnit) {
  try {
    const cutoff = cleanupCutoff(amount, unit);
    const preview = browserRecordingCleanupPreview(state, cutoff);
    let clearedHistoryRecords = 0;
    for (const record of state.history) {
      if (record.audio_path && preview.eligiblePaths.has(record.audio_path) && Date.parse(record.audio_finished_at) < cutoff) {
        record.audio_path = null;
        clearedHistoryRecords += 1;
      }
    }
    return Promise.resolve({
      deleted_files: preview.eligible_files,
      cleared_history_records: clearedHistoryRecords,
      freed_bytes: preview.eligible_bytes,
    });
  } catch (error) {
    return Promise.reject(error);
  }
}

function retryHistoryRecord(state: BrowserPreviewState, id: string) {
  const index = state.history.findIndex((record) => record.id === id);
  const record = state.history[index];
  if (!record) {
    return Promise.reject<HistoryRecord>(new Error("History record not found"));
  }
  if (
    !record.workflow_error
    || !record.audio_path
    || record.raw_text.trim() !== ""
    || record.corrected_text.trim() !== ""
    || record.pasted_text.trim() !== ""
  ) {
    return Promise.reject<HistoryRecord>(new Error("This history record is not eligible for retry"));
  }

  return new Promise<HistoryRecord>((resolve) => {
    window.setTimeout(() => {
      const updatedRecord: HistoryRecord = {
        ...record,
        asr_task_id: `preview-retry-${record.id}`,
        raw_text: "这是一段因为网络连接中断而未能完成识别的录音，现在已经通过手动重试成功转写。",
        corrected_text: "这是一段因网络连接中断而未能完成识别的录音，现在已通过手动重试成功转写。",
        pasted_text: "",
        correction_error: null,
        injection_error: null,
        workflow_error: null,
        asr_duration_ms: 860,
        service_audio_duration_ms: 8_400,
        total_duration_ms: 1_240,
      };
      state.history[index] = updatedRecord;
      resolve(clonePreview(updatedRecord));
    }, 900);
  });
}

function listenAudioLevel(state: BrowserPreviewState, handler: (sample: AudioLevelSample) => void) {
  const recordingRevision = state.status.mode === "recording" ? state.status.revision : 0;
  if (recordingRevision === 0) {
    handler({ level: 0, recording_revision: 0 });
    return Promise.resolve(() => undefined);
  }
  const levels = [0.2, 0.4, 0.6, 0.7, 0.8, 0.9, 0.6, 0.4];
  let index = 0;
  const emitNextLevel = () => {
    handler({ level: levels[index % levels.length], recording_revision: recordingRevision });
    index += 1;
  };
  emitNextLevel();
  const timer = window.setInterval(emitNextLevel, 100);
  return Promise.resolve(() => window.clearInterval(timer));
}

function browserRecordingCleanupPreview(state: BrowserPreviewState, cutoff: number): RecordingCleanupPreview & { eligiblePaths: Set<string> } {
  const recordingPaths = new Set<string>();
  const candidatePaths = new Set<string>();
  const protectedPaths = new Set<string>();
  for (const record of state.history) {
    if (!record.audio_path) continue;
    recordingPaths.add(record.audio_path);
    if (Date.parse(record.audio_finished_at) < cutoff) {
      candidatePaths.add(record.audio_path);
    } else {
      protectedPaths.add(record.audio_path);
    }
  }
  const eligiblePaths = new Set([...candidatePaths].filter((path) => !protectedPaths.has(path)));
  return {
    recording_files: recordingPaths.size,
    recording_bytes: recordingPaths.size * state.recordingBytes,
    eligible_files: eligiblePaths.size,
    eligible_bytes: eligiblePaths.size * state.recordingBytes,
    eligiblePaths,
  };
}
