import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { AppConfig, AudioInputDevice, AudioOutputDevice, ConfigImportReport, DataDirInfo, HistoryRecord, InputStats, RecordingCleanupUnit, VadTestStatus, WorkflowStatus } from "../types";
import type { CorrectionSection, Page } from "../domain/navigation";
import type { PermissionRequestState } from "../domain/permissions";
import { appLanguage, translations } from "../domain/i18n";
import { requiresAccessibilityPermission, supportsFnLongPressTrigger } from "../domain/platform";
import { emptyStatus, latestWorkflowStatus, subscribeToWorkflowStatus } from "../domain/workflow";
import { formatByteCount } from "../domain/historyMaintenance";
import NavButton from "../components/NavButton";
import InputStatsCard from "../components/InputStatsCard";
import PermissionGuide from "../components/PermissionGuide";
import HomePage from "../pages/HomePage";
import HistoryRecordsPage from "../pages/HistoryRecordsPage";
import ModelsPage from "../pages/ModelsPage";
import CorrectionPage from "../pages/CorrectionPage";
import SettingsPage from "../pages/SettingsPage";
import { accessibilityPermissionGranted, applyFnTrigger, cancelCurrentWorkflow as cancelCurrentWorkflowCommand, chooseDataDir, cleanupRecordingFiles as cleanupRecordingFilesCommand, copyTextToClipboard, deleteHistoryRecord as deleteHistoryRecordCommand, exportConfig as exportConfigCommand, getAppVersion, getDataDir, getStatus, getVadTestStatus, hideMainWindow, importConfig as importConfigCommand, inputMonitoringPermissionGranted, listenConfigCloseRequested, listenConfigUpdated, listenHistoryUpdated, listenVadTestStatus, listenWorkflowStatus, loadAudioInputDevices, loadAudioOutputDevices, loadConfig, loadHistory, loadStats, openAccessibilitySettings, openAppDir, openGitHubRepository, openInputMonitoringSettings, previewRecordingCleanup as previewRecordingCleanupCommand, requestAccessibilityPermission, requestInputMonitoringPermission as requestInputMonitoringPermissionCommand, requestMicrophonePermission as requestMicrophonePermissionCommand, resetDataDir as resetDataDirCommand, restartSystemAudioService as restartSystemAudioServiceCommand, retryHistoryRecord as retryHistoryRecordCommand, saveConfig, setDataDir as setDataDirCommand, startVadTest as startVadTestCommand, stopVadTest as stopVadTestCommand, toggleRecording as toggleRecordingCommand, updateVadTestSettings as updateVadTestSettingsCommand } from "./tauriApi";

const appIconUrl = new URL("../assets/app-icon.png", import.meta.url).href;
const recentHistoryLimit = 6;
const historyPageSize = 20;
const noticeDurationMs = 5_000;

interface QueuedNotice {
  id: number;
  message: string;
}

type PendingConfigAction =
  | { kind: "page"; page: Page; correctionSection?: CorrectionSection }
  | { kind: "close" };

export default function MainApp() {
  const vadSettingsRequestRef = useRef(0);
  const vadTestStartingRef = useRef(false);
  const needsAccessibilityPermission = requiresAccessibilityPermission();
  const [page, setPage] = useState<Page>("home");
  const [correctionSection, setCorrectionSection] = useState<CorrectionSection>("requirements");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);
  const [vadTestStatus, setVadTestStatus] = useState<VadTestStatus>({
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
    noise_window_ms: 2000,
    revision: 0,
    error: null,
  });
  const [history, setHistory] = useState<HistoryRecord[]>([]);
  const [stats, setStats] = useState<InputStats | null>(null);
  const [dataDir, setDataDirInfo] = useState<DataDirInfo | null>(null);
  const [audioDevices, setAudioDevices] = useState<AudioInputDevice[]>([]);
  const [audioOutputDevices, setAudioOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [audioInputDevicesChecked, setAudioInputDevicesChecked] = useState(false);
  const [audioDevicesRefreshing, setAudioDevicesRefreshing] = useState(false);
  const [audioServiceRestarting, setAudioServiceRestarting] = useState(false);
  const [historyPageRecords, setHistoryPageRecords] = useState<HistoryRecord[]>([]);
  const [historyPageIndex, setHistoryPageIndex] = useState(0);
  const [historyHasOlder, setHistoryHasOlder] = useState(false);
  const [noticeQueue, setNoticeQueue] = useState<QueuedNotice[]>([]);
  const [busy, setBusy] = useState(false);
  const [savingConfig, setSavingConfig] = useState(false);
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);
  const [inputMonitoringGranted, setInputMonitoringGranted] = useState<boolean | null>(null);
  const [inputMonitoringPermission, setInputMonitoringPermission] = useState<PermissionRequestState>("unknown");
  const [microphonePermission, setMicrophonePermission] = useState<PermissionRequestState>("unknown");
  const [showPermissionGuide, setShowPermissionGuide] = useState(false);
  const [pendingConfigAction, setPendingConfigAction] = useState<PendingConfigAction | null>(null);
  const [configImportReport, setConfigImportReport] = useState<ConfigImportReport | null>(null);
  const [appVersion, setAppVersion] = useState("");
  const configRef = useRef<AppConfig | null>(null);
  const savedConfigRef = useRef<AppConfig | null>(null);
  const pageRef = useRef(page);
  const contentScrollRef = useRef<HTMLDivElement | null>(null);
  const unsavedDialogRef = useRef<HTMLElement | null>(null);
  const unsavedCancelButtonRef = useRef<HTMLButtonElement | null>(null);
  const focusBeforeUnsavedDialogRef = useRef<HTMLElement | null>(null);
  const historyPageIndexRef = useRef(historyPageIndex);
  const nextNoticeIdRef = useRef(0);
  const currentNotice = noticeQueue[0] ?? null;

  const setNotice = useCallback((message: string) => {
    const normalized = message.trim();
    if (!normalized) {
      return;
    }
    nextNoticeIdRef.current += 1;
    const notice = { id: nextNoticeIdRef.current, message: normalized };
    setNoticeQueue((queue) => [...queue, notice]);
  }, []);

  const dismissNotice = useCallback((noticeId: number) => {
    setNoticeQueue((queue) => queue[0]?.id === noticeId ? queue.slice(1) : queue);
  }, []);

  async function refreshHistory() {
    const records = await loadHistory(recentHistoryLimit);
    setHistory(records);
  }

  async function refreshStats() {
    setStats(await loadStats());
  }

  async function refreshAudioDevices() {
    setAudioDevicesRefreshing(true);
    try {
      const [inputResult, outputResult] = await Promise.allSettled([
        loadAudioInputDevices(),
        loadAudioOutputDevices(),
      ]);
      if (inputResult.status === "fulfilled") {
        setAudioDevices(inputResult.value);
        setAudioInputDevicesChecked(true);
      }
      if (outputResult.status === "fulfilled") {
        setAudioOutputDevices(outputResult.value);
      }
      const errors = [inputResult, outputResult]
        .filter((result): result is PromiseRejectedResult => result.status === "rejected")
        .map((result) => String(result.reason));
      if (errors.length > 0) {
        throw new Error(errors.join("; "));
      }
    } finally {
      setAudioDevicesRefreshing(false);
    }
  }

  async function restartSystemAudioService() {
    setAudioServiceRestarting(true);
    setBusy(true);
    try {
      const restarted = await restartSystemAudioServiceCommand();
      if (!restarted) {
        setNotice(text.notices.audioServiceRestartCancelled);
        return;
      }
      try {
        await refreshAudioDevices();
        setNotice(text.notices.audioServiceRestarted);
      } catch (error) {
        setNotice(text.notices.audioServiceRestartedRefreshFailed(String(error)));
      }
    } catch (error) {
      setNotice(text.notices.audioServiceRestartFailed(String(error)));
    } finally {
      setAudioServiceRestarting(false);
      setBusy(false);
    }
  }

  async function startVadTest() {
    if (!config || busy || status.mode !== "idle" || vadTestStartingRef.current) {
      return;
    }
    vadTestStartingRef.current = true;
    const request = ++vadSettingsRequestRef.current;
    try {
      const nextStatus = await startVadTestCommand(config.audio);
      if (request === vadSettingsRequestRef.current) {
        setVadTestStatus((current) => nextStatus.revision > current.revision ? nextStatus : current);
      }
    } catch (error) {
      if (request === vadSettingsRequestRef.current) {
        setVadTestStatus((current) => ({ ...current, mode: "error", error: String(error), revision: current.revision + 1 }));
        setNotice(String(error));
      }
    } finally {
      vadTestStartingRef.current = false;
    }
  }

  async function updateVadTestSettings(noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number) {
    const request = ++vadSettingsRequestRef.current;
    try {
      const nextStatus = await updateVadTestSettingsCommand(noiseMarginDb, confirmationMs, noiseWindowMs);
      if (request === vadSettingsRequestRef.current) {
        setVadTestStatus((current) => nextStatus.revision > current.revision ? nextStatus : current);
      }
    } catch (error) {
      if (request === vadSettingsRequestRef.current) {
        setNotice(String(error));
      }
    }
  }

  async function stopVadTest() {
    vadSettingsRequestRef.current += 1;
    try {
      const nextStatus = await stopVadTestCommand();
      setVadTestStatus((current) => nextStatus.revision > current.revision ? nextStatus : current);
    } catch (error) {
      setNotice(String(error));
    }
  }

  async function loadHistoryPage(pageIndex: number) {
    const records = await loadHistory(historyPageSize + 1, pageIndex * historyPageSize);
    const pageRecords = records.slice(0, historyPageSize);
    historyPageIndexRef.current = pageIndex;
    setHistoryPageIndex(pageIndex);
    setHistoryPageRecords(pageRecords);
    setHistoryHasOlder(records.length > historyPageSize);
    return pageRecords;
  }

  async function refreshHistoryViews(fallbackFromEmptyPage = false) {
    await refreshHistory();
    if (pageRef.current !== "history") {
      return;
    }
    const currentPage = historyPageIndexRef.current;
    const records = await loadHistoryPage(currentPage);
    if (fallbackFromEmptyPage && records.length === 0 && currentPage > 0) {
      await loadHistoryPage(currentPage - 1);
    }
  }

  async function refreshAll() {
    const loadedConfig = await loadConfig();
    applyLoadedConfig(loadedConfig);
    const [historyResult, statsResult, dataDirResult, accessibilityResult] = await Promise.allSettled([
      loadHistory(recentHistoryLimit),
      loadStats(),
      getDataDir(),
      accessibilityPermissionGranted(),
    ]);

    if (historyResult.status === "fulfilled") {
      setHistory(historyResult.value);
    }
    if (statsResult.status === "fulfilled") {
      setStats(statsResult.value);
    }
    if (dataDirResult.status === "fulfilled") {
      setDataDirInfo(dataDirResult.value);
    }
    if (accessibilityResult.status === "fulfilled") {
      setAccessibilityGranted(accessibilityResult.value);
      if (needsAccessibilityPermission && !accessibilityResult.value) {
        setShowPermissionGuide(true);
      }
    }

    const errors = [historyResult, statsResult, dataDirResult, accessibilityResult]
      .filter((result): result is PromiseRejectedResult => result.status === "rejected")
      .map((result) => String(result.reason));
    if (errors.length > 0) {
      throw new Error(errors.join("; "));
    }
  }

  useEffect(() => {
    const stopStatusSubscription = subscribeToWorkflowStatus({
      listen: listenWorkflowStatus,
      getSnapshot: getStatus,
      onStatus: (nextStatus) => setStatus((current) => latestWorkflowStatus(current, nextStatus)),
      onError: (error) => setNotice(String(error)),
    });
    const unlistenVad = listenVadTestStatus((nextStatus) => {
      setVadTestStatus((current) => nextStatus.revision > current.revision ? nextStatus : current);
    });
    getVadTestStatus()
      .then((nextStatus) => setVadTestStatus((current) => nextStatus.revision > current.revision ? nextStatus : current))
      .catch(() => undefined);
    getAppVersion().then(setAppVersion).catch(() => setAppVersion(""));
    refreshAll().catch((error) => setNotice(String(error)));
    refreshAudioDevices().catch((error) => setNotice(String(error)));
    const unlistenHistory = listenHistoryUpdated(() => {
      refreshHistoryViews(true).catch((error) => setNotice(String(error)));
      refreshStats().catch((error) => setNotice(String(error)));
    });
    const unlistenConfig = listenConfigUpdated((updatedConfig) => {
      if (isConfigDirty(configRef.current, savedConfigRef.current)) {
        savedConfigRef.current = updatedConfig;
        setSavedConfig(updatedConfig);
        return;
      }
      applyLoadedConfig(updatedConfig);
    });
    const unlistenClose = listenConfigCloseRequested(() => {
      requestWindowClose();
    });
    return () => {
      stopStatusSubscription();
      unlistenHistory.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
      unlistenClose.then((fn) => fn());
      unlistenVad.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    pageRef.current = page;
    if (contentScrollRef.current) {
      contentScrollRef.current.scrollTop = 0;
    }
    if (page !== "history") {
      return;
    }
    loadHistoryPage(historyPageIndex).catch((error) => setNotice(String(error)));
  }, [page]);

  useEffect(() => {
    historyPageIndexRef.current = historyPageIndex;
  }, [historyPageIndex]);

  useEffect(() => {
    if (!needsAccessibilityPermission || accessibilityGranted !== false) {
      return;
    }

    const timer = window.setInterval(() => {
      refreshAccessibility().catch(() => undefined);
    }, 2000);
    return () => window.clearInterval(timer);
  }, [accessibilityGranted]);

  useEffect(() => {
    if (!currentNotice) {
      return;
    }

    const timer = window.setTimeout(() => dismissNotice(currentNotice.id), noticeDurationMs);
    return () => window.clearTimeout(timer);
  }, [currentNotice?.id, dismissNotice]);

  useEffect(() => {
    if (!pendingConfigAction) {
      const previousFocus = focusBeforeUnsavedDialogRef.current;
      focusBeforeUnsavedDialogRef.current = null;
      previousFocus?.focus();
      return;
    }
    if (!focusBeforeUnsavedDialogRef.current && document.activeElement instanceof HTMLElement) {
      focusBeforeUnsavedDialogRef.current = document.activeElement;
    }
    if (busy) {
      return;
    }
    const frame = window.requestAnimationFrame(() => unsavedCancelButtonRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [pendingConfigAction, busy]);

  useEffect(() => {
    if (!config?.system.fn_long_press_enabled) {
      setInputMonitoringGranted(null);
      setInputMonitoringPermission("unknown");
      return;
    }
    if (inputMonitoringPermission !== "unknown") {
      return;
    }
    refreshInputMonitoring().catch((error) => setNotice(String(error)));
  }, [config?.system.fn_long_press_enabled, inputMonitoringPermission]);

  const language = appLanguage(config);
  const text = translations[language];
  const hasUnsavedChanges = isConfigDirty(config, savedConfig);

  function applyLoadedConfig(nextConfig: AppConfig) {
    configRef.current = nextConfig;
    savedConfigRef.current = nextConfig;
    setConfig(nextConfig);
    setSavedConfig(nextConfig);
  }

  function changeConfig(nextConfig: AppConfig) {
    configRef.current = nextConfig;
    setConfig(nextConfig);
  }

  async function persistConfig(nextConfig: AppConfig, successMessage?: string) {
    setBusy(true);
    setSavingConfig(true);
    try {
      const persistedConfig = await saveConfig(nextConfig);
      savedConfigRef.current = persistedConfig;
      setSavedConfig(persistedConfig);
      if (!isConfigDirty(configRef.current, nextConfig)) {
        configRef.current = persistedConfig;
        setConfig(persistedConfig);
      }
      setNotice(successMessage ?? text.notices.configSaved);
      return persistedConfig;
    } catch (error) {
      setNotice(String(error));
      return null;
    } finally {
      setSavingConfig(false);
      setBusy(false);
    }
  }

  async function save(nextConfig: AppConfig, successMessage?: string) {
    await persistConfig(nextConfig, successMessage);
  }

  function requestPageChange(nextPage: Page) {
    if (nextPage === page) {
      return;
    }
    if (isConfigDirty(configRef.current, savedConfigRef.current)) {
      setPendingConfigAction({ kind: "page", page: nextPage });
      return;
    }
    setPage(nextPage);
  }

  function requestCorrectionSection(nextSection: CorrectionSection) {
    if (page === "correction") {
      setCorrectionSection(nextSection);
      return;
    }
    if (isConfigDirty(configRef.current, savedConfigRef.current)) {
      setPendingConfigAction({ kind: "page", page: "correction", correctionSection: nextSection });
      return;
    }
    setCorrectionSection(nextSection);
    setPage("correction");
  }

  function requestWindowClose() {
    if (isConfigDirty(configRef.current, savedConfigRef.current)) {
      setPendingConfigAction({ kind: "close" });
      return;
    }
    hideMainWindow().catch((error) => setNotice(String(error)));
  }

  async function savePendingConfig() {
    const action = pendingConfigAction;
    const nextConfig = configRef.current;
    if (!action || !nextConfig) {
      return;
    }
    const saved = await persistConfig(nextConfig);
    if (!saved) {
      return;
    }
    setPendingConfigAction(null);
    completePendingConfigAction(action);
  }

  async function discardPendingConfig() {
    const action = pendingConfigAction;
    if (!action) {
      return;
    }
    const persisted = savedConfigRef.current;
    if (persisted) {
      applyLoadedConfig(persisted);
      if (supportsFnLongPressTrigger()) {
        setBusy(true);
        try {
          await applyFnTrigger(
            persisted.system.fn_long_press_enabled ?? false,
            persisted.system.fn_long_press_duration_ms ?? 200,
          );
        } catch (error) {
          setNotice(String(error));
        } finally {
          setBusy(false);
        }
      }
    }
    setPendingConfigAction(null);
    completePendingConfigAction(action);
  }

  function completePendingConfigAction(action: PendingConfigAction) {
    if (action.kind === "page") {
      if (action.correctionSection) {
        setCorrectionSection(action.correctionSection);
      }
      setPage(action.page);
      return;
    }
    hideMainWindow().catch((error) => setNotice(String(error)));
  }

  async function toggleRecording() {
    setBusy(true);
    try {
      const nextStatus = await toggleRecordingCommand();
      setStatus((current) => latestWorkflowStatus(current, nextStatus));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function cancelWorkflow() {
    setBusy(true);
    try {
      const nextStatus = await cancelCurrentWorkflowCommand();
      setStatus((current) => latestWorkflowStatus(current, nextStatus));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function refreshAccessibility() {
    const granted = await accessibilityPermissionGranted();
    setAccessibilityGranted(granted);
    return granted;
  }

  async function refreshInputMonitoring() {
    setInputMonitoringPermission("checking");
    const granted = await inputMonitoringPermissionGranted();
    if (!granted) {
      setInputMonitoringGranted(false);
      setInputMonitoringPermission("denied");
      return false;
    }
    if (configRef.current?.system.fn_long_press_enabled) {
      let active = false;
      try {
        active = await applyCurrentFnTrigger(true);
      } catch {
        active = false;
      }
      setInputMonitoringGranted(active);
      setInputMonitoringPermission(active ? "granted" : "denied");
      if (!active) {
        setNotice(text.permission.inputMonitoringNotice);
      }
      return active;
    }
    setInputMonitoringGranted(true);
    setInputMonitoringPermission("granted");
    return true;
  }

  async function openAccessibilityPermission() {
    try {
      const granted = await requestAccessibilityPermission();
      const latest = granted || await refreshAccessibility();
      setAccessibilityGranted(latest);
      if (!latest) {
        await openAccessibilitySettings();
        setNotice(text.permission.accessibilityNotice);
      } else {
        setNotice(text.permission.accessibilityGranted);
      }
    } catch (error) {
      setNotice(String(error));
    }
  }

  async function requestInputMonitoringPermission() {
    setInputMonitoringPermission("checking");
    try {
      const granted = await requestInputMonitoringPermissionCommand();
      const latest = granted || await refreshInputMonitoring();
      if (!latest) {
        setInputMonitoringGranted(false);
        setInputMonitoringPermission("denied");
        await openInputMonitoringSettings();
        setNotice(text.permission.inputMonitoringNotice);
        return;
      }

      if (configRef.current?.system.fn_long_press_enabled) {
        let active = false;
        try {
          active = await applyCurrentFnTrigger(true);
        } catch {
          active = false;
        }
        setInputMonitoringGranted(active);
        setInputMonitoringPermission(active ? "granted" : "denied");
        if (!active) {
          await openInputMonitoringSettings();
          setNotice(text.permission.inputMonitoringNotice);
          return;
        }
      } else {
        setInputMonitoringGranted(true);
        setInputMonitoringPermission("granted");
      }
      setNotice(text.permission.inputMonitoringGranted);
    } catch (error) {
      setInputMonitoringGranted(false);
      setInputMonitoringPermission("denied");
      setNotice(String(error));
    }
  }

  async function applyCurrentFnTrigger(enabled = configRef.current?.system.fn_long_press_enabled ?? false) {
    const currentConfig = configRef.current;
    if (!currentConfig) {
      return false;
    }
    await applyFnTrigger(enabled, currentConfig.system.fn_long_press_duration_ms ?? 200);
    return true;
  }

  async function requestMicrophonePermission() {
    setMicrophonePermission("checking");
    try {
      const granted = await requestMicrophonePermissionCommand();
      setMicrophonePermission(granted ? "granted" : "denied");
      setNotice(granted ? text.permission.microphoneGranted : text.permission.microphoneDenied);
    } catch (error) {
      setMicrophonePermission("denied");
      setNotice(String(error));
    }
  }

  async function copyHistoryText(value: string, label: string) {
    if (!value.trim()) {
      setNotice(text.notices.emptyCopy(label));
      return;
    }

    try {
      await copyTextToClipboard(value);
      setNotice(text.notices.copied(label));
    } catch (error) {
      setNotice(String(error));
    }
  }

  async function deleteHistory(record: HistoryRecord) {
    setBusy(true);
    try {
      const result = await deleteHistoryRecordCommand(record.id);
      await refreshHistoryViews(true);
      setNotice(result.deleted_records === 0
        ? text.notices.historyDeleteNotFound
        : text.notices.historyDeleted(
          result.deleted_records,
          result.deleted_audio_files,
          formatByteCount(result.freed_bytes),
        ));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function retryHistory(record: HistoryRecord) {
    setBusy(true);
    try {
      const updatedRecord = await retryHistoryRecordCommand(record.id);
      setHistory((records) => replaceHistoryRecord(records, updatedRecord));
      setHistoryPageRecords((records) => replaceHistoryRecord(records, updatedRecord));
      await refreshStats().catch(() => undefined);
      setNotice(text.notices.historyRetrySucceeded);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setNotice(text.notices.historyRetryFailed(message));
    } finally {
      setBusy(false);
    }
  }

  async function cleanupOldRecordingFiles(amount: number, unit: RecordingCleanupUnit) {
    setBusy(true);
    try {
      const result = await cleanupRecordingFilesCommand(amount, unit);
      await refreshHistoryViews();
      setNotice(result.deleted_files === 0 && result.cleared_history_records === 0
        ? text.notices.noRecordingsCleaned
        : text.notices.recordingsCleaned(
          result.deleted_files,
          result.cleared_history_records,
          formatByteCount(result.freed_bytes),
        ));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function exportCurrentConfig() {
    const currentConfig = configRef.current;
    if (!currentConfig) {
      return;
    }

    setBusy(true);
    try {
      const path = await exportConfigCommand(currentConfig);
      setNotice(text.notices.configExported(path));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function refreshDataAfterDirectoryChange(info: DataDirInfo) {
    setDataDirInfo(info);
    await Promise.all([
      refreshHistory(),
      refreshStats(),
      pageRef.current === "history" ? loadHistoryPage(historyPageIndexRef.current) : Promise.resolve(),
    ]);
  }

  async function changeDataDir() {
    setBusy(true);
    try {
      const selectedPath = await chooseDataDir();
      if (!selectedPath) {
        return;
      }
      const info = await setDataDirCommand(selectedPath);
      await refreshDataAfterDirectoryChange(info);
      setNotice(info.cleanup_warning ?? text.notices.dataDirChanged(info.path));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function resetDataDir() {
    setBusy(true);
    try {
      const info = await resetDataDirCommand();
      await refreshDataAfterDirectoryChange(info);
      setNotice(info.cleanup_warning ?? text.notices.dataDirChanged(info.path));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function openGitHubRepositoryPage() {
    try {
      await openGitHubRepository();
    } catch (error) {
      setNotice(String(error));
    }
  }

  async function importConfigFile(file: File) {
    setBusy(true);
    try {
      const raw = await file.text();
      const result = await importConfigCommand(raw);
      applyLoadedConfig(result.config);
      setConfigImportReport(result.report);
      setNotice(text.notices.configImported);
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  function openHistoryPage() {
    setPage("history");
    loadHistoryPage(0).catch((error) => setNotice(String(error)));
  }

  function changeHistoryPage(nextPage: number) {
    loadHistoryPage(nextPage).catch((error) => setNotice(String(error)));
  }

  function handleUnsavedDialogKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      setPendingConfigAction(null);
      return;
    }
    if (event.key !== "Tab") {
      return;
    }

    const buttons = Array.from(unsavedDialogRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
    if (buttons.length === 0) {
      return;
    }
    const firstButton = buttons[0];
    const lastButton = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === firstButton) {
      event.preventDefault();
      lastButton.focus();
    } else if (!event.shiftKey && document.activeElement === lastButton) {
      event.preventDefault();
      firstButton.focus();
    }
  }

  const canSave = Boolean(config) && !busy;
  const canSaveChanges = canSave && hasUnsavedChanges;
  const canMutateHistory = !busy && status.mode !== "starting" && status.mode !== "recording" && status.mode !== "processing";
  const canChangeDataDir = canSave && canMutateHistory;
  const showSaveBar = Boolean(config) && (hasUnsavedChanges || page === "models" || page === "correction" || page === "settings");
  const correctionNavItems: Array<{ section: CorrectionSection; label: string }> = [
    { section: "requirements", label: text.correction.requirementsNav },
    { section: "dictionary", label: text.correction.dictionaryNav },
    { section: "rules", label: text.correction.rulesNav },
    { section: "prompt", label: text.correction.promptNav },
  ];

  return (
    <div className="app-shell">
      <aside className={page === "correction" ? "sidebar correction-active" : "sidebar"}>
        <div className="brand">
          <img className="brand-mark" src={appIconUrl} alt="" aria-hidden="true" />
          <div className="brand-copy">
            <div className="brand-title">BoltScribe</div>
            <div className="brand-subtitle">{text.appSubtitle}</div>
            {appVersion ? <div className="brand-version">{text.common.version(appVersion)}</div> : null}
          </div>
          <button
            className="brand-github-button"
            type="button"
            onClick={() => { void openGitHubRepositoryPage(); }}
            aria-label={text.nav.openGitHubRepository}
            title={text.nav.openGitHubRepository}
          >
            <GitHubIcon />
          </button>
        </div>
        <nav className="sidebar-nav" aria-label={language === "zh-CN" ? "主导航" : "Main navigation"}>
          <NavButton current={page} page="home" onClick={requestPageChange} label={text.nav.home} />
          <NavButton current={page} page="history" onClick={requestPageChange} label={text.nav.history} />
          <NavButton current={page} page="models" onClick={requestPageChange} label={text.nav.models} />
          <div className="correction-nav-group">
            <div className="correction-nav-primary">
              <NavButton
                current={page}
                page="correction"
                onClick={requestPageChange}
                label={text.nav.correction}
                ariaExpanded={page === "correction"}
              />
              <label className="nav-toggle" title={text.correction.enabled}>
                <span className="visually-hidden">{text.correction.enabled}</span>
                <input
                  type="checkbox"
                  role="switch"
                  checked={config?.correction.enabled ?? false}
                  disabled={!config || busy}
                  onChange={(event) => {
                    if (!config) return;
                    changeConfig({ ...config, correction: { ...config.correction, enabled: event.target.checked } });
                  }}
                />
              </label>
            </div>
            {page === "correction" ? (
              <nav className="correction-subnav" aria-label={text.correction.navigationLabel}>
                {correctionNavItems.map((item) => (
                  <button
                    key={item.section}
                    className={correctionSection === item.section ? "correction-subnav-button active" : "correction-subnav-button"}
                    type="button"
                    aria-current={correctionSection === item.section ? "page" : undefined}
                    onClick={() => setCorrectionSection(item.section)}
                  >
                    {item.label}
                  </button>
                ))}
              </nav>
            ) : null}
          </div>
          <NavButton current={page} page="settings" onClick={requestPageChange} label={text.nav.settings} />
        </nav>
        <div className="sidebar-footer">
          <InputStatsCard stats={stats} text={text} />
          <button className="ghost-button" onClick={() => { void openAppDir(); }}>
            {text.nav.openDataDir}
          </button>
        </div>
      </aside>

      <main className="content">
        <div className={page === "correction" ? "content-scroll correction-content" : "content-scroll"} ref={contentScrollRef}>
          {needsAccessibilityPermission && accessibilityGranted === false ? (
            <div className="permission-banner">
              <div>
                <strong>{text.permission.bannerTitle}</strong>
                <span>{text.permission.bannerText}</span>
              </div>
              <div className="permission-actions">
                <button className="secondary small" onClick={() => refreshAccessibility().catch((error) => setNotice(String(error)))}>{text.permission.recheck}</button>
                <button className="secondary small" onClick={() => setShowPermissionGuide(true)}>{text.home.permissionGuide}</button>
                <button className="secondary small" onClick={openAccessibilityPermission}>{text.permission.openSettings}</button>
              </div>
            </div>
          ) : null}
          {!config ? (
            <section className="panel">
              <h1>{language === "zh-CN" ? "正在加载配置" : "Loading configuration"}</h1>
            </section>
          ) : null}
          {config && page === "home" ? (
            <HomePage
              config={config}
              status={status}
              busy={busy}
              history={history}
              inputDevicesChecked={audioInputDevicesChecked}
              hasInputDevice={audioDevices.length > 0}
              audioDevicesRefreshing={audioDevicesRefreshing}
              onToggle={toggleRecording}
              onCancel={cancelWorkflow}
              onOpenPermissionGuide={() => setShowPermissionGuide(true)}
              onRefreshAudioDevices={() => { void refreshAudioDevices().catch((error) => setNotice(String(error))); }}
              onRefreshHistory={refreshHistory}
              onOpenHistoryPage={openHistoryPage}
              onOpenModels={() => requestPageChange("models")}
              onOpenCorrectionSection={requestCorrectionSection}
              onCopyHistory={copyHistoryText}
              onRetryHistory={retryHistory}
              onDeleteHistory={deleteHistory}
              canRetryHistory={canMutateHistory}
              canDeleteHistory={canMutateHistory}
              language={language}
              text={text}
            />
          ) : null}
          {config && page === "history" ? (
            <HistoryRecordsPage
              history={historyPageRecords}
              pageIndex={historyPageIndex}
              pageSize={historyPageSize}
              hasOlder={historyHasOlder}
              onRefresh={() => changeHistoryPage(historyPageIndex)}
              onPreviousPage={() => changeHistoryPage(Math.max(0, historyPageIndex - 1))}
              onNextPage={() => changeHistoryPage(historyPageIndex + 1)}
              onCopyHistory={copyHistoryText}
              onRetryHistory={retryHistory}
              onDeleteHistory={deleteHistory}
              canRetryHistory={canMutateHistory}
              canDeleteHistory={canMutateHistory}
              text={text}
            />
          ) : null}
          {config && page === "models" ? (
            <ModelsPage config={config} onChange={changeConfig} onSaveConfig={save} onNotice={setNotice} canSave={canSave} text={text} />
          ) : null}
          {config && page === "correction" ? (
            <CorrectionPage config={config} onChange={changeConfig} section={correctionSection} text={text} />
          ) : null}
          {config && page === "settings" ? (
            <SettingsPage
              config={config}
              audioDevices={audioDevices}
              audioOutputDevices={audioOutputDevices}
              dataDir={dataDir}
              audioDevicesRefreshing={audioDevicesRefreshing}
              audioServiceRestarting={audioServiceRestarting}
              onChange={changeConfig}
              onExportConfig={() => { void exportCurrentConfig(); }}
              onImportConfig={(file) => { void importConfigFile(file); }}
              onOpenDataDir={() => { void openAppDir(); }}
              onChooseDataDir={() => { void changeDataDir(); }}
              onResetDataDir={() => { void resetDataDir(); }}
              onCleanupRecordingFiles={cleanupOldRecordingFiles}
              onPreviewRecordingCleanup={previewRecordingCleanupCommand}
              onRefreshAudioDevices={() => { void refreshAudioDevices().catch((error) => setNotice(String(error))); }}
              onRestartAudioService={() => { void restartSystemAudioService(); }}
              vadTestStatus={vadTestStatus}
              onStartVadTest={() => { void startVadTest(); }}
              onUpdateVadTestSettings={(noiseMarginDb, confirmationMs, noiseWindowMs) => {
                void updateVadTestSettings(noiseMarginDb, confirmationMs, noiseWindowMs);
              }}
              onStopVadTest={() => { void stopVadTest(); }}
              inputMonitoringGranted={inputMonitoringGranted}
              inputMonitoringPermission={inputMonitoringPermission}
              onRefreshInputMonitoring={() => { void refreshInputMonitoring().catch((error) => setNotice(String(error))); }}
              onRequestInputMonitoring={() => { void requestInputMonitoringPermission(); }}
              onApplyFnTrigger={(enabled) => { void applyCurrentFnTrigger(enabled).catch((error) => setNotice(String(error))); }}
              importReport={configImportReport}
              canSave={canSave}
              canChangeDataDir={canChangeDataDir}
              canCleanupRecordings={canChangeDataDir}
              canRestartAudioService={canMutateHistory}
              text={text}
            />
          ) : null}
        </div>
        {showSaveBar ? (
          <div className={hasUnsavedChanges ? "config-save-bar dirty" : "config-save-bar"}>
            <div className="config-save-status" role="status" aria-live="polite">
              <span className="config-save-dot" aria-hidden="true" />
              <span>{hasUnsavedChanges ? text.unsaved.statusPending : text.unsaved.statusSaved}</span>
            </div>
            <button
              className="primary small"
              type="button"
              disabled={!canSaveChanges}
              onClick={() => {
                const currentConfig = configRef.current;
                if (currentConfig) {
                  void save(currentConfig);
                }
              }}
            >
              {savingConfig ? text.common.saving : text.common.save}
            </button>
          </div>
        ) : null}
      </main>
      {pendingConfigAction ? (
        <div className="modal-backdrop" role="presentation">
          <section
            ref={unsavedDialogRef}
            className="unsaved-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="unsaved-title"
            aria-describedby="unsaved-message"
            onKeyDown={handleUnsavedDialogKeyDown}
          >
            <div>
              <h1 id="unsaved-title">{text.unsaved.title}</h1>
              <p id="unsaved-message">{text.unsaved.message}</p>
            </div>
            <div className="modal-footer">
              <button ref={unsavedCancelButtonRef} className="secondary small" type="button" onClick={() => setPendingConfigAction(null)} disabled={busy}>{text.unsaved.cancel}</button>
              <button className="secondary small" type="button" onClick={() => { void discardPendingConfig(); }} disabled={busy}>{text.unsaved.discard}</button>
              <button className="primary small" type="button" onClick={() => { void savePendingConfig(); }} disabled={busy || !hasUnsavedChanges}>{text.unsaved.save}</button>
            </div>
          </section>
        </div>
      ) : null}
      {showPermissionGuide ? (
        <PermissionGuide
          accessibilityGranted={accessibilityGranted}
          requiresAccessibility={needsAccessibilityPermission}
          microphonePermission={microphonePermission}
          onClose={() => setShowPermissionGuide(false)}
          onRefreshAccessibility={() => refreshAccessibility().catch((error) => setNotice(String(error)))}
          onOpenAccessibility={openAccessibilityPermission}
          onRequestMicrophone={requestMicrophonePermission}
          text={text}
        />
      ) : null}
      {currentNotice ? (
        <div className="toast" key={currentNotice.id} role="status" aria-live="polite" aria-atomic="true">
          <span>{currentNotice.message}</span>
          <button className="toast-close" type="button" onClick={() => dismissNotice(currentNotice.id)} aria-label={text.notices.closeNotice}>×</button>
        </div>
      ) : null}
    </div>
  );
}

function GitHubIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path
        fill="currentColor"
        d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.1.68-.22.68-.49 0-.24-.01-1.05-.02-1.91-2.78.62-3.37-1.21-3.37-1.21-.45-1.19-1.11-1.5-1.11-1.5-.91-.64.07-.63.07-.63 1 .07 1.53 1.06 1.53 1.06.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.56-1.14-4.56-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.71 0 0 .84-.28 2.75 1.05A9.3 9.3 0 0 1 12 6.96c.85 0 1.7.12 2.5.34 1.91-1.33 2.75-1.05 2.75-1.05.55 1.41.2 2.45.1 2.71.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.81-4.57 5.07.36.32.68.94.68 1.9 0 1.37-.01 2.48-.01 2.81 0 .27.18.59.69.49A10.17 10.17 0 0 0 22 12.25C22 6.58 17.52 2 12 2Z"
      />
    </svg>
  );
}

function isConfigDirty(config: AppConfig | null, savedConfig: AppConfig | null) {
  if (!config || !savedConfig) {
    return false;
  }
  return JSON.stringify(config) !== JSON.stringify(savedConfig);
}

function replaceHistoryRecord(records: HistoryRecord[], updatedRecord: HistoryRecord) {
  return records.map((record) => record.id === updatedRecord.id ? updatedRecord : record);
}
