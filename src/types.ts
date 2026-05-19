export interface AppConfig {
  hotkey: string;
  hotkeys: string[];
  hotkey_enabled: boolean[];
  audio: AudioConfig;
  asr: AsrConfig;
  llm: LlmConfig;
  correction: CorrectionConfig;
  ui: UiConfig;
  retention: RetentionConfig;
  system: SystemConfig;
}

export interface AudioConfig {
  input_device_mode: "system_default" | "manual" | string;
  input_device_id: string | null;
  input_device_name: string | null;
  output_volume_ducking: OutputVolumeDuckingConfig;
}

export interface OutputVolumeDuckingConfig {
  enabled: boolean;
  reduction_percent: number;
  device_name_whitelist: string[];
}

export interface AudioInputDevice {
  id: string;
  name: string;
  is_default: boolean;
  platform: string;
}

export interface AudioOutputDevice {
  id: string;
  name: string;
  is_default: boolean;
  platform: string;
  supports_volume_control: boolean;
  supports_mute_control: boolean;
}

export interface AsrConfig {
  provider: string;
  app_key: string;
  access_key: string;
  resource_id: string;
  stream_url: string;
  submit_url: string;
  query_url: string;
  language: string;
}

export interface LlmConfig {
  provider: "openai" | "volc_ark" | "custom" | string;
  api_format: "chat_completions" | "responses" | string;
  endpoint: string;
  api_key: string;
  model: string;
  provider_settings: LlmProviderSettings[];
  race_enabled: boolean;
  race_models: string[];
  race_targets: RaceModelTarget[];
  system_prompt: string;
  temperature: number;
  timeout_secs: number;
  thinking_enabled: boolean;
  thinking_effort: "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | string;
  max_output_tokens: number | null;
  model_presets: ModelPreset[];
}

export interface ModelPreset {
  provider: string;
  model: string;
}

export interface LlmProviderSettings {
  provider: string;
  endpoint: string;
  api_format: string;
  api_key: string;
}

export interface RaceModelTarget {
  provider: string;
  model: string;
}

export interface UiConfig {
  app_language: "zh-CN" | "en-US" | string;
  recording_overlay_scale: number;
  recording_overlay_offset_x: number;
  recording_overlay_offset_y: number;
}

export interface RetentionConfig {
  max_history_records: number;
  max_storage_bytes: number;
}

export interface SystemConfig {
  launch_at_login: boolean;
  hide_dock_icon: boolean;
}

export interface CorrectionConfig {
  enabled: boolean;
  user_requirements: string;
  prompt_template: string;
  variables: PromptVariable[];
  correction_rules: CorrectionRule[];
  dictionary: DictionaryEntry[];
}

export interface PromptVariable {
  name: string;
  value: string;
}

export interface DictionaryEntry {
  term: string;
  aliases: string[];
  note: string;
}

export interface CorrectionRule {
  source: string;
  target: string;
  note: string;
}

export interface ConfigImportReport {
  format: string | null;
  version: number | null;
  missing_fields: string[];
  unknown_fields: string[];
  invalid_fields: string[];
  notes: string[];
}

export interface ConfigImportResult {
  config: AppConfig;
  report: ConfigImportReport;
}

export interface WorkflowStatus {
  mode: "idle" | "recording" | "processing" | "error";
  message: string;
  current_audio_path: string | null;
  last_record_id: string | null;
}

export interface HistoryRecord {
  id: string;
  created_at: string;
  audio_path: string;
  asr_provider: string;
  asr_task_id: string | null;
  audio_started_at: string;
  audio_finished_at: string;
  audio_sample_rate: number;
  audio_channels: number;
  audio_sample_count: number;
  raw_text: string;
  corrected_text: string;
  pasted_text: string;
  correction_enabled: boolean;
  correction_error: string | null;
  correction_logs: LlmCallLog[];
  injection_error: string | null;
  workflow_error: string | null;
  asr_duration_ms: number | null;
  service_audio_duration_ms: number | null;
  total_duration_ms: number;
}

export interface InputStats {
  total_character_count: number;
  total_audio_duration_ms: number;
  average_chars_per_minute: number;
  daily: DailyInputStats[];
}

export interface DailyInputStats {
  date: string;
  record_count: number;
  character_count: number;
  audio_duration_ms: number;
}

export interface LlmUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  reasoning_tokens: number | null;
}

export interface LlmCallLog {
  provider: string;
  model: string;
  api_format: string;
  endpoint: string;
  duration_ms: number;
  success: boolean;
  request_id: string | null;
  finish_reason: string | null;
  usage: LlmUsage | null;
  error: string | null;
}
