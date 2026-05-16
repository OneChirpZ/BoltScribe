import { useEffect, useRef, useState } from "react";
import type { AppConfig, HistoryRecord, InputStats, WorkflowStatus } from "../types";
import type { Page } from "../domain/navigation";
import type { PermissionRequestState } from "../domain/permissions";
import { appLanguage, translations } from "../domain/i18n";
import { emptyStatus } from "../domain/workflow";
import NavButton from "../components/NavButton";
import InputStatsCard from "../components/InputStatsCard";
import PermissionGuide from "../components/PermissionGuide";
import HomePage from "../pages/HomePage";
import HistoryRecordsPage from "../pages/HistoryRecordsPage";
import ModelsPage from "../pages/ModelsPage";
import CorrectionPage from "../pages/CorrectionPage";
import SettingsPage from "../pages/SettingsPage";
import { accessibilityPermissionGranted, copyTextToClipboard, getStatus, hideMainWindow, listenConfigCloseRequested, listenConfigUpdated, listenHistoryUpdated, listenWorkflowStatus, loadConfig, loadHistory, loadStats, openAccessibilitySettings, openAppDir, requestAccessibilityPermission, requestMicrophonePermission as requestMicrophonePermissionCommand, saveConfig, toggleRecording as toggleRecordingCommand } from "./tauriApi";

const appIconUrl = new URL("../assets/app-icon.png", import.meta.url).href;
const recentHistoryLimit = 6;
const historyPageSize = 20;

type PendingConfigAction =
  | { kind: "page"; page: Page }
  | { kind: "close" };

export default function MainApp() {
  const [page, setPage] = useState<Page>("home");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);
  const [history, setHistory] = useState<HistoryRecord[]>([]);
  const [stats, setStats] = useState<InputStats | null>(null);
  const [historyPageRecords, setHistoryPageRecords] = useState<HistoryRecord[]>([]);
  const [historyPageIndex, setHistoryPageIndex] = useState(0);
  const [historyHasOlder, setHistoryHasOlder] = useState(false);
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);
  const [microphonePermission, setMicrophonePermission] = useState<PermissionRequestState>("unknown");
  const [showPermissionGuide, setShowPermissionGuide] = useState(false);
  const [pendingConfigAction, setPendingConfigAction] = useState<PendingConfigAction | null>(null);
  const configRef = useRef<AppConfig | null>(null);
  const savedConfigRef = useRef<AppConfig | null>(null);

  async function refreshHistory() {
    const records = await loadHistory(recentHistoryLimit);
    setHistory(records);
  }

  async function refreshStats() {
    setStats(await loadStats());
  }

  async function loadHistoryPage(pageIndex: number) {
    const records = await loadHistory(historyPageSize + 1, pageIndex * historyPageSize);
    setHistoryPageIndex(pageIndex);
    setHistoryPageRecords(records.slice(0, historyPageSize));
    setHistoryHasOlder(records.length > historyPageSize);
  }

  async function refreshAll() {
    const [loadedConfig, loadedStatus, records, loadedStats, hasAccessibility] = await Promise.all([
      loadConfig(),
      getStatus(),
      loadHistory(recentHistoryLimit),
      loadStats(),
      accessibilityPermissionGranted(),
    ]);
    applyLoadedConfig(loadedConfig);
    setStatus(loadedStatus);
    setHistory(records);
    setStats(loadedStats);
    setAccessibilityGranted(hasAccessibility);
    if (!hasAccessibility) {
      setShowPermissionGuide(true);
    }
  }

  useEffect(() => {
    refreshAll().catch((error) => setNotice(String(error)));
    const unlistenStatus = listenWorkflowStatus(setStatus);
    const unlistenHistory = listenHistoryUpdated(() => {
      refreshHistory().catch((error) => setNotice(String(error)));
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
      unlistenStatus.then((fn) => fn());
      unlistenHistory.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
      unlistenClose.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (page !== "history") {
      return;
    }
    loadHistoryPage(historyPageIndex).catch((error) => setNotice(String(error)));
  }, [page]);

  useEffect(() => {
    if (accessibilityGranted !== false) {
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
    try {
      const savedConfig = await saveConfig(nextConfig);
      applyLoadedConfig(savedConfig);
      setNotice(successMessage ?? text.notices.configSaved);
      return savedConfig;
    } catch (error) {
      setNotice(String(error));
      return null;
    } finally {
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

  function discardPendingConfig() {
    const action = pendingConfigAction;
    if (!action) {
      return;
    }
    const persisted = savedConfigRef.current;
    if (persisted) {
      applyLoadedConfig(persisted);
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

  function openHistoryPage() {
    setPage("history");
    loadHistoryPage(0).catch((error) => setNotice(String(error)));
  }

  function changeHistoryPage(nextPage: number) {
    loadHistoryPage(nextPage).catch((error) => setNotice(String(error)));
  }

  const canSave = Boolean(config) && !busy;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img className="brand-mark" src={appIconUrl} alt="" aria-hidden="true" />
          <div>
            <div className="brand-title">BoltScribe</div>
            <div className="brand-subtitle">{text.appSubtitle}</div>
          </div>
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
        {accessibilityGranted === false ? (
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
            onToggle={toggleRecording}
            onOpenPermissionGuide={() => setShowPermissionGuide(true)}
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
          <ModelsPage config={config} onChange={changeConfig} onSave={() => save(config)} onSaveConfig={save} onNotice={setNotice} canSave={canSave} text={text} />
        ) : null}
        {config && page === "correction" ? (
          <CorrectionPage config={config} onChange={changeConfig} onSave={() => save(config)} canSave={canSave} text={text} />
        ) : null}
        {config && page === "settings" ? (
          <SettingsPage config={config} onChange={changeConfig} onSave={() => save(config)} canSave={canSave} text={text} />
        ) : null}
      </main>
      {pendingConfigAction ? (
        <div className="modal-backdrop" role="presentation">
          <section className="unsaved-dialog" role="dialog" aria-modal="true" aria-labelledby="unsaved-title">
            <div>
              <h1 id="unsaved-title">{text.unsaved.title}</h1>
              <p>{text.unsaved.message}</p>
            </div>
            <div className="modal-footer">
              <button className="secondary small" type="button" onClick={() => setPendingConfigAction(null)} disabled={busy}>{text.unsaved.cancel}</button>
              <button className="secondary small" type="button" onClick={discardPendingConfig} disabled={busy}>{text.unsaved.discard}</button>
              <button className="primary small" type="button" onClick={() => { void savePendingConfig(); }} disabled={busy || !hasUnsavedChanges}>{text.unsaved.save}</button>
            </div>
          </section>
        </div>
      ) : null}
      {showPermissionGuide ? (
        <PermissionGuide
          accessibilityGranted={accessibilityGranted}
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

function isConfigDirty(config: AppConfig | null, savedConfig: AppConfig | null) {
  if (!config || !savedConfig) {
    return false;
  }
  return JSON.stringify(config) !== JSON.stringify(savedConfig);
}
