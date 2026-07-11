import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { AppConfig, AudioInputDevice, AudioOutputDevice, ConfigImportReport, DataDirInfo, HistoryRecord, InputStats, WorkflowStatus } from "../types";
import type { Page } from "../domain/navigation";
import type { PermissionRequestState } from "../domain/permissions";
import { appLanguage, translations } from "../domain/i18n";
import { requiresAccessibilityPermission, supportsFnLongPressTrigger } from "../domain/platform";
import { emptyStatus } from "../domain/workflow";
import NavButton from "../components/NavButton";
import InputStatsCard from "../components/InputStatsCard";
import PermissionGuide from "../components/PermissionGuide";
import HomePage from "../pages/HomePage";
import HistoryRecordsPage from "../pages/HistoryRecordsPage";
import ModelsPage from "../pages/ModelsPage";
import CorrectionPage from "../pages/CorrectionPage";
import SettingsPage from "../pages/SettingsPage";
import { accessibilityPermissionGranted, applyFnTrigger, chooseDataDir, copyTextToClipboard, exportConfig as exportConfigCommand, getAppVersion, getDataDir, getStatus, hideMainWindow, importConfig as importConfigCommand, inputMonitoringPermissionGranted, listenConfigCloseRequested, listenConfigUpdated, listenHistoryUpdated, listenWorkflowStatus, loadAudioInputDevices, loadAudioOutputDevices, loadConfig, loadHistory, loadStats, openAccessibilitySettings, openAppDir, openGitHubRepository, openInputMonitoringSettings, requestAccessibilityPermission, requestInputMonitoringPermission as requestInputMonitoringPermissionCommand, requestMicrophonePermission as requestMicrophonePermissionCommand, resetDataDir as resetDataDirCommand, saveConfig, setDataDir as setDataDirCommand, toggleRecording as toggleRecordingCommand } from "./tauriApi";

const appIconUrl = new URL("../assets/app-icon.png", import.meta.url).href;
const recentHistoryLimit = 6;
const historyPageSize = 20;

type PendingConfigAction =
  | { kind: "page"; page: Page }
  | { kind: "close" };

export default function MainApp() {
  const needsAccessibilityPermission = requiresAccessibilityPermission();
  const [page, setPage] = useState<Page>("home");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);
  const [history, setHistory] = useState<HistoryRecord[]>([]);
  const [stats, setStats] = useState<InputStats | null>(null);
  const [dataDir, setDataDirInfo] = useState<DataDirInfo | null>(null);
  const [audioDevices, setAudioDevices] = useState<AudioInputDevice[]>([]);
  const [audioOutputDevices, setAudioOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [audioInputDevicesChecked, setAudioInputDevicesChecked] = useState(false);
  const [audioDevicesRefreshing, setAudioDevicesRefreshing] = useState(false);
  const [historyPageRecords, setHistoryPageRecords] = useState<HistoryRecord[]>([]);
  const [historyPageIndex, setHistoryPageIndex] = useState(0);
  const [historyHasOlder, setHistoryHasOlder] = useState(false);
  const [notice, setNotice] = useState("");
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

  async function loadHistoryPage(pageIndex: number) {
    const records = await loadHistory(historyPageSize + 1, pageIndex * historyPageSize);
    historyPageIndexRef.current = pageIndex;
    setHistoryPageIndex(pageIndex);
    setHistoryPageRecords(records.slice(0, historyPageSize));
    setHistoryHasOlder(records.length > historyPageSize);
  }

  async function refreshAll() {
    const loadedConfig = await loadConfig();
    applyLoadedConfig(loadedConfig);
    const [statusResult, historyResult, statsResult, dataDirResult, accessibilityResult] = await Promise.allSettled([
      getStatus(),
      loadHistory(recentHistoryLimit),
      loadStats(),
      getDataDir(),
      accessibilityPermissionGranted(),
    ]);

    if (statusResult.status === "fulfilled") {
      setStatus(statusResult.value);
    }
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

    const errors = [statusResult, historyResult, statsResult, dataDirResult, accessibilityResult]
      .filter((result): result is PromiseRejectedResult => result.status === "rejected")
      .map((result) => String(result.reason));
    if (errors.length > 0) {
      throw new Error(errors.join("; "));
    }
  }

  useEffect(() => {
    getAppVersion().then(setAppVersion).catch(() => setAppVersion(""));
    refreshAll().catch((error) => setNotice(String(error)));
    refreshAudioDevices().catch((error) => setNotice(String(error)));
    const unlistenStatus = listenWorkflowStatus(setStatus);
    const unlistenHistory = listenHistoryUpdated(() => {
      refreshHistory().catch((error) => setNotice(String(error)));
      refreshStats().catch((error) => setNotice(String(error)));
      if (pageRef.current === "history") {
        loadHistoryPage(historyPageIndexRef.current).catch((error) => setNotice(String(error)));
      }
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
      unlistenStatus.then((fn) => fn());
      unlistenHistory.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
      unlistenClose.then((fn) => fn());
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
    if (!notice) {
      return;
    }

    const timer = window.setTimeout(() => setNotice(""), 10000);
    return () => window.clearTimeout(timer);
  }, [notice]);

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
      setPage(action.page);
      return;
    }
    hideMainWindow().catch((error) => setNotice(String(error)));
  }

  async function toggleRecording() {
    setBusy(true);
    try {
      const nextStatus = await toggleRecordingCommand();
      setStatus(nextStatus);
      setNotice("");
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
  const canChangeDataDir = canSave && status.mode !== "recording" && status.mode !== "processing";
  const showSaveBar = Boolean(config) && (page === "models" || page === "correction" || page === "settings");

  return (
    <div className="app-shell">
      <aside className="sidebar">
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
        <nav>
          <NavButton current={page} page="home" onClick={requestPageChange} label={text.nav.home} />
          <NavButton current={page} page="history" onClick={requestPageChange} label={text.nav.history} />
          <NavButton current={page} page="models" onClick={requestPageChange} label={text.nav.models} />
          <NavButton current={page} page="correction" onClick={requestPageChange} label={text.nav.correction} />
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
        <div className="content-scroll" ref={contentScrollRef}>
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
              onOpenPermissionGuide={() => setShowPermissionGuide(true)}
              onRefreshAudioDevices={() => { void refreshAudioDevices().catch((error) => setNotice(String(error))); }}
              onRefreshHistory={refreshHistory}
              onOpenHistoryPage={openHistoryPage}
              onCopyHistory={copyHistoryText}
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
              text={text}
            />
          ) : null}
          {config && page === "models" ? (
            <ModelsPage config={config} onChange={changeConfig} onSaveConfig={save} onNotice={setNotice} canSave={canSave} text={text} />
          ) : null}
          {config && page === "correction" ? (
            <CorrectionPage config={config} onChange={changeConfig} text={text} />
          ) : null}
          {config && page === "settings" ? (
            <SettingsPage
              config={config}
              audioDevices={audioDevices}
              audioOutputDevices={audioOutputDevices}
              dataDir={dataDir}
              audioDevicesRefreshing={audioDevicesRefreshing}
              onChange={changeConfig}
              onExportConfig={() => { void exportCurrentConfig(); }}
              onImportConfig={(file) => { void importConfigFile(file); }}
              onOpenDataDir={() => { void openAppDir(); }}
              onChooseDataDir={() => { void changeDataDir(); }}
              onResetDataDir={() => { void resetDataDir(); }}
              onRefreshAudioDevices={() => { void refreshAudioDevices().catch((error) => setNotice(String(error))); }}
              inputMonitoringGranted={inputMonitoringGranted}
              inputMonitoringPermission={inputMonitoringPermission}
              onRefreshInputMonitoring={() => { void refreshInputMonitoring().catch((error) => setNotice(String(error))); }}
              onRequestInputMonitoring={() => { void requestInputMonitoringPermission(); }}
              onApplyFnTrigger={(enabled) => { void applyCurrentFnTrigger(enabled).catch((error) => setNotice(String(error))); }}
              importReport={configImportReport}
              canSave={canSave}
              canChangeDataDir={canChangeDataDir}
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
      {notice ? (
        <div className="toast" role="status" aria-live="polite">
          <span>{notice}</span>
          <button className="toast-close" type="button" onClick={() => setNotice("")} aria-label={text.notices.closeNotice}>×</button>
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
