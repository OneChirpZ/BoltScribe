import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppConfig, AudioInputDevice, ConfigImportResult, HistoryRecord, InputStats, WorkflowStatus } from "../types";

export function loadConfig() {
  return invoke<AppConfig>("load_config");
}

export function saveConfig(config: AppConfig) {
  return invoke<AppConfig>("save_config", { config });
}

export function exportConfig(config: AppConfig) {
  return invoke<string>("export_config", { config });
}

export function importConfig(raw: string) {
  return invoke<ConfigImportResult>("import_config", { raw });
}

export function loadAudioInputDevices() {
  return invoke<AudioInputDevice[]>("load_audio_input_devices");
}

export function getAppVersion() {
  return getVersion();
}

export function loadHistory(limit: number, offset = 0) {
  return invoke<HistoryRecord[]>("load_history", { limit, offset });
}

export function loadStats() {
  return invoke<InputStats>("load_stats");
}

export function getStatus() {
  return invoke<WorkflowStatus>("get_status");
}

export function toggleRecording() {
  return invoke<WorkflowStatus>("toggle_recording");
}

export function cancelCurrentWorkflow() {
  return invoke<WorkflowStatus>("cancel_current_workflow");
}

export function openAppDir() {
  return invoke("open_app_dir");
}

export function hideMainWindow() {
  return invoke("hide_main_window");
}

export function accessibilityPermissionGranted() {
  return invoke<boolean>("accessibility_permission_granted");
}

export function requestAccessibilityPermission() {
  return invoke<boolean>("request_accessibility_permission");
}

export function openAccessibilitySettings() {
  return invoke("open_accessibility_settings");
}

export function requestMicrophonePermission() {
  return invoke<boolean>("request_microphone_permission");
}

export function copyTextToClipboard(text: string) {
  return invoke("copy_text_to_clipboard", { text });
}

export function listenWorkflowStatus(handler: (status: WorkflowStatus) => void) {
  return listen<WorkflowStatus>("workflow://status", (event) => handler(event.payload));
}

export function listenHistoryUpdated(handler: () => void) {
  return listen("history://updated", handler);
}

export function listenConfigUpdated(handler: (config: AppConfig) => void) {
  return listen<AppConfig>("config://updated", (event) => handler(event.payload));
}

export function listenConfigCloseRequested(handler: () => void) {
  return listen("config://close-requested", handler);
}
