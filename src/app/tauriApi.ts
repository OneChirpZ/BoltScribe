import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppConfig, AudioInputDevice, AudioOutputDevice, ConfigImportResult, DataDirInfo, HistoryRecord, InputStats, WorkflowStatus } from "../types";

export function loadConfig() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewConfig));
  }
  return invoke<AppConfig>("load_config");
}

export function saveConfig(config: AppConfig) {
  if (browserPreviewEnabled()) {
    previewConfig = clonePreview(config);
    return Promise.resolve(clonePreview(previewConfig));
  }
  return invoke<AppConfig>("save_config", { config });
}

export function exportConfig(config: AppConfig) {
  if (browserPreviewEnabled()) {
    return Promise.resolve(`browser-preview-config-${config.ui.app_language}.json`);
  }
  return invoke<string>("export_config", { config });
}

export function importConfig(raw: string) {
  if (browserPreviewEnabled()) {
    const parsed = JSON.parse(raw) as AppConfig;
    previewConfig = clonePreview(parsed);
    return Promise.resolve({
      config: clonePreview(previewConfig),
      report: {
        format: "browser-preview",
        version: null,
        missing_fields: [],
        unknown_fields: [],
        invalid_fields: [],
        notes: [],
      },
    });
  }
  return invoke<ConfigImportResult>("import_config", { raw });
}

export function loadAudioInputDevices() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewAudioInputDevices));
  }
  return invoke<AudioInputDevice[]>("load_audio_input_devices");
}

export function loadAudioOutputDevices() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewAudioOutputDevices));
  }
  return invoke<AudioOutputDevice[]>("load_audio_output_devices");
}

export function getAppVersion() {
  if (browserPreviewEnabled()) {
    return Promise.resolve("1.1.0");
  }
  return getVersion();
}

export function loadHistory(limit: number, offset = 0) {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewHistory.slice(offset, offset + limit)));
  }
  return invoke<HistoryRecord[]>("load_history", { limit, offset });
}

export function loadStats() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewStats));
  }
  return invoke<InputStats>("load_stats");
}

export function getStatus() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewStatus));
  }
  return invoke<WorkflowStatus>("get_status");
}

export function toggleRecording() {
  if (browserPreviewEnabled()) {
    previewStatus = previewStatus.mode === "recording"
      ? { mode: "idle", message: "就绪", current_audio_path: null, last_record_id: null }
      : { mode: "recording", message: "正在录音，再次按快捷键停止", current_audio_path: null, last_record_id: null };
    return Promise.resolve(clonePreview(previewStatus));
  }
  return invoke<WorkflowStatus>("toggle_recording");
}

export function cancelCurrentWorkflow() {
  if (browserPreviewEnabled()) {
    previewStatus = { mode: "idle", message: "已取消本次转写", current_audio_path: null, last_record_id: null };
    return Promise.resolve(clonePreview(previewStatus));
  }
  return invoke<WorkflowStatus>("cancel_current_workflow");
}

export function openAppDir() {
  if (browserPreviewEnabled()) {
    return Promise.resolve();
  }
  return invoke("open_app_dir");
}

export function openGitHubRepository() {
  if (browserPreviewEnabled()) {
    window.open("https://github.com/OneChirpZ/BoltScribe", "_blank", "noopener,noreferrer");
    return Promise.resolve();
  }
  return invoke("open_github_repository");
}

export function getDataDir() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(clonePreview(previewDataDirInfo));
  }
  return invoke<DataDirInfo>("get_data_dir");
}

export function chooseDataDir() {
  if (browserPreviewEnabled()) {
    return Promise.resolve("D:\\BoltScribeData");
  }
  return invoke<string | null>("choose_data_dir");
}

export function setDataDir(path: string) {
  if (browserPreviewEnabled()) {
    previewDataDirInfo = {
      path,
      default_path: previewDataDirInfo.default_path,
      is_default: path === previewDataDirInfo.default_path,
      cleanup_warning: null,
    };
    return Promise.resolve(clonePreview(previewDataDirInfo));
  }
  return invoke<DataDirInfo>("set_data_dir", { path });
}

export function resetDataDir() {
  if (browserPreviewEnabled()) {
    previewDataDirInfo = clonePreview(previewDefaultDataDirInfo);
    return Promise.resolve(clonePreview(previewDataDirInfo));
  }
  return invoke<DataDirInfo>("reset_data_dir");
}

export function hideMainWindow() {
  if (browserPreviewEnabled()) {
    return Promise.resolve();
  }
  return invoke("hide_main_window");
}

export function accessibilityPermissionGranted() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(true);
  }
  return invoke<boolean>("accessibility_permission_granted");
}

export function requestAccessibilityPermission() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(true);
  }
  return invoke<boolean>("request_accessibility_permission");
}

export function openAccessibilitySettings() {
  if (browserPreviewEnabled()) {
    return Promise.resolve();
  }
  return invoke("open_accessibility_settings");
}

export function inputMonitoringPermissionGranted() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(true);
  }
  return invoke<boolean>("input_monitoring_permission_granted");
}

export function requestInputMonitoringPermission() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(true);
  }
  return invoke<boolean>("request_input_monitoring_permission");
}

export function applyFnTrigger(enabled: boolean, longPressDurationMs: number) {
  if (browserPreviewEnabled()) {
    return Promise.resolve();
  }
  return invoke("apply_fn_trigger", { enabled, longPressDurationMs });
}

export function openInputMonitoringSettings() {
  if (browserPreviewEnabled()) {
    return Promise.resolve();
  }
  return invoke("open_input_monitoring_settings");
}

export function requestMicrophonePermission() {
  if (browserPreviewEnabled()) {
    return Promise.resolve(true);
  }
  return invoke<boolean>("request_microphone_permission");
}

export function copyTextToClipboard(text: string) {
  if (browserPreviewEnabled()) {
    return navigator.clipboard?.writeText(text) ?? Promise.resolve();
  }
  return invoke("copy_text_to_clipboard", { text });
}

export function listenWorkflowStatus(handler: (status: WorkflowStatus) => void) {
  if (browserPreviewEnabled()) {
    handler(clonePreview(previewStatus));
    return Promise.resolve(() => undefined);
  }
  return listen<WorkflowStatus>("workflow://status", (event) => handler(event.payload));
}

export function listenHistoryUpdated(handler: () => void) {
  if (browserPreviewEnabled()) {
    return Promise.resolve(() => undefined);
  }
  return listen("history://updated", handler);
}

export function listenConfigUpdated(handler: (config: AppConfig) => void) {
  if (browserPreviewEnabled()) {
    return Promise.resolve(() => undefined);
  }
  return listen<AppConfig>("config://updated", (event) => handler(event.payload));
}

export function listenConfigCloseRequested(handler: () => void) {
  if (browserPreviewEnabled()) {
    return Promise.resolve(() => undefined);
  }
  return listen("config://close-requested", handler);
}

function browserPreviewEnabled() {
  return import.meta.env.DEV && typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);
}

function clonePreview<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

let previewStatus: WorkflowStatus = {
  mode: "idle",
  message: "就绪",
  current_audio_path: null,
  last_record_id: null,
};

let previewConfig: AppConfig = {
  hotkey: "PageUp",
  hotkeys: ["PageUp", "CmdOrCtrl+Shift+Space"],
  hotkey_enabled: [true, true],
  audio: {
    input_device_mode: "system_default",
    input_device_id: null,
    input_device_name: null,
    output_volume_ducking: {
      enabled: true,
      mute_instead_of_reduce: false,
      reduction_percent: 70,
      device_name_whitelist: ["Audient iD4"],
      sound_source_hotkey_fallback_enabled: false,
      sound_source_toggle_mute_hotkey: "Cmd+Opt+Ctrl+A",
    },
  },
  asr: {
    provider: "volcengine",
    auth_mode: "api_key",
    app_key: "1575344452",
    access_key: "preview-access-key",
    resource_id: "volc.seedasr.sauc.duration",
    stream_url: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream",
    submit_url: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/submit",
    query_url: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/query",
    language: "zh-CN",
  },
  llm: {
    provider: "openai",
    api_format: "chat_completions",
    endpoint: "https://api.openai.com/v1",
    api_key: "preview-api-key",
    model: "gpt-5.4-mini",
    provider_settings: [],
    race_enabled: true,
    race_models: ["gpt-5.4-mini", "doubao-seed-2-0-lite-260428"],
    race_targets: [
      { provider: "openai", model: "gpt-5.4-mini" },
      { provider: "volc_ark", model: "doubao-seed-2-0-lite-260428" },
    ],
    system_prompt: "你是语音输入文本优化器。",
    temperature: 1,
    timeout_secs: 12,
    thinking_enabled: false,
    thinking_effort: "none",
    max_output_tokens: 4000,
    model_presets: [],
  },
  correction: {
    enabled: true,
    user_requirements: "",
    prompt_template: "纠错任务：\\n{{raw_text}}",
    variables: [],
    dictionary_text: "",
    correction_rules_text: "",
    correction_rules: [],
    dictionary: [],
  },
  ui: {
    app_language: "zh-CN",
    recording_overlay_scale: 0.5,
    recording_overlay_offset_x: 0,
    recording_overlay_offset_y: 0,
  },
  retention: {
    max_history_records: 500,
    max_storage_bytes: 2147483648,
  },
  system: {
    launch_at_login: false,
    hide_dock_icon: false,
    tray_left_click_recording_enabled: true,
    fn_long_press_enabled: false,
    fn_long_press_duration_ms: 200,
  },
};

const previewDefaultDataDirInfo: DataDirInfo = {
  path: "C:\\Users\\Preview\\AppData\\Roaming\\BoltScribe",
  default_path: "C:\\Users\\Preview\\AppData\\Roaming\\BoltScribe",
  is_default: true,
  cleanup_warning: null,
};

let previewDataDirInfo: DataDirInfo = clonePreview(previewDefaultDataDirInfo);

const previewAudioInputDevices: AudioInputDevice[] = [
  { id: "system-default", name: "MacBook Pro 麦克风", is_default: true, platform: "macos" },
  { id: "external-mic", name: "External USB Mic", is_default: false, platform: "macos" },
];

const previewAudioOutputDevices: AudioOutputDevice[] = [
  { id: "audient-id4", name: "Audient iD4", is_default: true, platform: "macos", supports_volume_control: false, supports_mute_control: true },
  { id: "macbook-speakers", name: "MacBook Pro 扬声器", is_default: false, platform: "macos", supports_volume_control: true, supports_mute_control: true },
  { id: "headphones", name: "External Headphones", is_default: false, platform: "macos", supports_volume_control: true, supports_mute_control: true },
];

const previewHistory: HistoryRecord[] = [
  previewHistoryRecord("1", "2026-05-20T00:50:21+08:00", "在你修改这样的前端展示时，不需要每次重复打包，然后让我来帮你检查，而是你直接把这个前端在浏览器等内容当中渲染出来，直接你来做截图和视觉检查。"),
  previewHistoryRecord("2", "2026-05-20T00:49:16+08:00", "增加最小宽度限制，避免窗口被缩得太小，导致内容挤占和排版错乱。"),
  previewHistoryRecord("3", "2026-05-20T00:48:21+08:00", "优化语音输入浮窗大小滑动条和快捷键 Command 按钮的排版。"),
];

const previewStats: InputStats = {
  total_character_count: 17909,
  total_audio_duration_ms: 6360000,
  average_chars_per_minute: 168,
  daily: Array.from({ length: 70 }, (_, index) => {
    const date = new Date(Date.UTC(2026, 2, 12 + index));
    const active = index > 60 ? index - 60 : 0;
    return {
      date: date.toISOString().slice(0, 10),
      record_count: active,
      character_count: active * 420,
      audio_duration_ms: active * 42000,
    };
  }),
};

function previewHistoryRecord(id: string, created_at: string, text: string): HistoryRecord {
  return {
    id,
    created_at,
    audio_path: "",
    asr_provider: "volcengine",
    asr_task_id: null,
    audio_started_at: created_at,
    audio_finished_at: created_at,
    audio_sample_rate: 48000,
    audio_channels: 1,
    audio_sample_count: 0,
    raw_text: text,
    corrected_text: text,
    pasted_text: text,
    correction_enabled: true,
    correction_error: null,
    correction_logs: [],
    injection_error: null,
    workflow_error: null,
    asr_duration_ms: 820,
    service_audio_duration_ms: 2700,
    total_duration_ms: 3611,
  };
}
