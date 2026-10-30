import type {
  AppConfig,
  AudioInputDevice,
  AudioOutputDevice,
  DataDirInfo,
  HistoryRecord,
  InputStats,
  VadTestStatus,
  WorkflowStatus,
} from "../../types";
import { defaultAsrConnectionRetry } from "../../domain/asrConnectionRetry";

export interface BrowserPreviewState {
  status: WorkflowStatus;
  config: AppConfig;
  vadTestStatus: VadTestStatus;
  defaultDataDirInfo: DataDirInfo;
  dataDirInfo: DataDirInfo;
  audioInputDevices: AudioInputDevice[];
  audioOutputDevices: AudioOutputDevice[];
  history: HistoryRecord[];
  stats: InputStats;
  recordingBytes: number;
}

export function clonePreview<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function createBrowserPreviewState(locationHref = browserLocationHref()): BrowserPreviewState {
  const location = new URL(locationHref);
  const language = location.searchParams.get("preview-language") === "en-US" ? "en-US" : "zh-CN";
  const overlayScaleParam = location.searchParams.get("preview-overlay-scale");
  const requestedOverlayScale = overlayScaleParam === null ? Number.NaN : Number(overlayScaleParam);
  const overlayScale = Number.isFinite(requestedOverlayScale)
    ? Math.min(1, Math.max(0.25, requestedOverlayScale))
    : 0.5;
  const defaultDataDirInfo: DataDirInfo = {
    path: "C:\\Users\\Preview\\AppData\\Roaming\\BoltScribe",
    default_path: "C:\\Users\\Preview\\AppData\\Roaming\\BoltScribe",
    is_default: true,
    cleanup_warning: null,
  };

  return {
    status: initialBrowserPreviewStatus(location),
    config: previewConfig(language, overlayScale),
    vadTestStatus: {
      mode: "idle",
      raw_voice_active: false,
      voice_active: false,
      level: -96,
      noise_calibrated: false,
      noise_floor: -96,
      trigger_threshold: -96,
      trigger_progress: 0,
      elapsed_ms: 0,
      remaining_ms: 60_000,
      noise_margin_db: 12,
      confirmation_ms: 480,
      noise_window_ms: 800,
      revision: 0,
      error: null,
    },
    defaultDataDirInfo,
    dataDirInfo: clonePreview(defaultDataDirInfo),
    audioInputDevices: [
      { id: "system-default", name: "MacBook Pro 麦克风", is_default: true, platform: "macos" },
      { id: "external-mic", name: "External USB Mic", is_default: false, platform: "macos" },
      { id: "capture-card", name: "USB2 Digital Audio", is_default: false, platform: "macos" },
    ],
    audioOutputDevices: [
      { id: "audient-id4", name: "Audient iD4", is_default: true, platform: "macos", supports_volume_control: false, supports_mute_control: true },
      { id: "macbook-speakers", name: "MacBook Pro 扬声器", is_default: false, platform: "macos", supports_volume_control: true, supports_mute_control: true },
      { id: "headphones", name: "External Headphones", is_default: false, platform: "macos", supports_volume_control: true, supports_mute_control: true },
    ],
    history: previewHistory(language),
    stats: previewStats(),
    recordingBytes: 256 * 1024,
  };
}

function browserLocationHref() {
  return typeof window === "undefined" ? "http://localhost/" : window.location.href;
}

function initialBrowserPreviewStatus(location: URL): WorkflowStatus {
  const mode = location.searchParams.get("preview-status");
  if (mode === "starting") {
    return {
      mode,
      stage: "starting",
      code: "starting_microphone",
      detail: null,
      message: "正在启动麦克风",
      current_audio_path: null,
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "recording") {
    return {
      mode,
      stage: "recording",
      code: "recording",
      detail: null,
      message: "正在录音，再次按快捷键停止",
      current_audio_path: null,
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "waiting_for_speech") {
    return {
      mode: "recording",
      stage: "waiting_for_speech",
      code: "waiting_for_speech",
      detail: null,
      message: "等待说话",
      current_audio_path: null,
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "file_asr_fallback") {
    return {
      mode: "processing",
      stage: "file_asr_fallback",
      code: "file_asr_fallback",
      detail: null,
      message: "正在回退到录音文件识别",
      current_audio_path: "/preview/recordings/fallback.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "recognizing") {
    return {
      mode: "processing",
      stage: "recognizing",
      code: "recognizing",
      detail: null,
      message: "录音已保存，正在进行语音识别",
      current_audio_path: "/preview/recordings/current.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "correcting") {
    return {
      mode: "processing",
      stage: mode,
      code: mode,
      detail: null,
      message: "语音识别完成，正在纠错",
      current_audio_path: "/preview/recordings/current.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "pasting") {
    return {
      mode: "processing",
      stage: mode,
      code: mode,
      detail: null,
      message: "正在粘贴文本",
      current_audio_path: "/preview/recordings/current.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "complete") {
    return {
      mode: "processing",
      stage: mode,
      code: "paste_complete",
      detail: null,
      message: "粘贴完成",
      current_audio_path: "/preview/recordings/current.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  if (mode === "error") {
    return {
      mode,
      stage: mode,
      code: "failed",
      detail: "Browser preview error",
      message: "处理失败",
      current_audio_path: "/preview/recordings/current.wav",
      last_record_id: null,
      revision: 1,
    };
  }
  return {
    mode: "idle",
    stage: "idle",
    code: "ready",
    detail: null,
    message: "就绪",
    current_audio_path: null,
    last_record_id: null,
    revision: 0,
  };
}

function previewConfig(language: "zh-CN" | "en-US", overlayScale: number): AppConfig {
  return {
    hotkey: "PageUp",
    hotkeys: ["PageUp", "CmdOrCtrl+Shift+Space"],
    hotkey_enabled: [true, true],
    audio: {
      input_device_mode: "manual",
      input_device_id: "external-mic",
      input_device_name: "External USB Mic",
      input_device_priority: [{ id: "external-mic", name: "External USB Mic" }],
      input_device_blacklist: [{ id: "capture-card", name: "USB2 Digital Audio" }],
      output_volume_ducking: {
        enabled: true,
        mute_instead_of_reduce: false,
        reduction_percent: 70,
        device_name_whitelist: ["Audient iD4"],
        sound_source_hotkey_fallback_enabled: false,
        sound_source_toggle_mute_hotkey: "Cmd+Opt+Ctrl+A",
      },
      voice_activity_detection: {
        enabled: false,
        noise_margin_db: 12,
        confirmation_ms: 480,
        noise_window_ms: 800,
        initial_silence_timeout_secs: 15,
      },
    },
    asr: {
      provider: "volcengine",
      auth_mode: "api_key",
      app_key: "preview-app-key",
      access_key: "preview-access-key",
      resource_id: "volc.seedasr.sauc.duration",
      stream_url: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream",
      submit_url: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/submit",
      query_url: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/query",
      language,
      connection_retry: { ...defaultAsrConnectionRetry },
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
      system_prompt: language === "en-US"
        ? "Polish the dictated text without changing its meaning."
        : "在不改变原意的前提下整理语音输入文本。",
      temperature: 1,
      timeout_secs: 12,
      fast_mode_enabled: true,
      thinking_enabled: false,
      thinking_effort: "none",
      max_output_tokens: 4000,
      model_presets: [],
    },
    correction: {
      enabled: true,
      user_requirements: language === "en-US"
        ? "Keep the original meaning, improve punctuation, and preserve dictionary terms."
        : "保留原意，修正错别字和标点，并保持词典术语的固定写法。",
      prompt_template: "纠错任务：\n用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n易错词纠正：\n{{correction_rules}}\n\n原始转写：\n{{raw_text}}",
      variables: [{ name: "scene", value: language === "en-US" ? "Everyday notes" : "日常记录" }],
      dictionary_text: "BoltScribe\nASR\nLLM",
      disabled_dictionary_terms: [],
      correction_rules_text: "\"Bolt Scribe\" -> \"BoltScribe\" # Product name\n\"voice to text\" -> \"dictation\" # Preferred term",
      correction_rules: [],
      dictionary: [],
    },
    ui: {
      app_language: language,
      theme: "system",
      recording_overlay_scale: overlayScale,
      recording_overlay_snap_range_percent: 200,
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
      show_menu_bar_icon: true,
      tray_left_click_recording_enabled: true,
      fn_long_press_enabled: false,
      fn_long_press_duration_ms: 200,
    },
  };
}

function previewHistory(language: "zh-CN" | "en-US"): HistoryRecord[] {
  return language === "en-US"
    ? [
        previewHistoryRecord("1", "2026-08-09T14:36:18+08:00", "Turn today's product discussion into three clear action items, keeping each owner and due date."),
        previewHistoryRecord("2", "2026-08-09T14:31:42+08:00", "Start recognition only after speech is detected, so accidental silence does not create unnecessary requests."),
        previewHistoryRecord("3", "2026-08-09T14:24:07+08:00", "Paste the polished transcript into the active document when processing is complete."),
      ]
    : [
        previewHistoryRecord("1", "2026-08-09T14:36:18+08:00", "请把今天的产品讨论整理成三条清晰的行动项，并保留负责人和截止时间。"),
        previewHistoryRecord("2", "2026-08-09T14:31:42+08:00", "检测到人声后再开始识别，避免安静时产生无效请求。"),
        previewHistoryRecord("3", "2026-08-09T14:24:07+08:00", "处理完成后，将整理好的文本粘贴到当前文档。"),
      ];
}

function previewStats(): InputStats {
  return {
    total_character_count: 18640,
    total_audio_duration_ms: 6540000,
    average_chars_per_minute: 171,
    daily: Array.from({ length: 70 }, (_, index) => {
      const date = new Date("2026-08-09T12:00:00+08:00");
      date.setHours(0, 0, 0, 0);
      date.setDate(date.getDate() - 69 + index);
      const active = index > 60 ? index - 60 : 0;
      return {
        date: previewDateKey(date),
        record_count: active,
        character_count: active * 420,
        audio_duration_ms: active * 42000,
      };
    }),
  };
}

function previewDateKey(date: Date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function previewHistoryRecord(id: string, created_at: string, text: string): HistoryRecord {
  return {
    id,
    created_at,
    audio_path: `/preview/recordings/${id}.wav`,
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
