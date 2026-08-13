import NavButton from "../components/NavButton";
import InputStatsCard from "../components/InputStatsCard";
import PermissionGuide from "../components/PermissionGuide";
import HomePage from "../pages/HomePage";
import HistoryRecordsPage from "../pages/HistoryRecordsPage";
import ModelsPage from "../pages/ModelsPage";
import CorrectionPage from "../pages/CorrectionPage";
import SettingsPage from "../pages/SettingsPage";
import { historyPageSize } from "./controllers/useHistoryController";
import { useMainAppController } from "./controllers/useMainAppController";

const appIconUrl = new URL("../assets/app-icon.png", import.meta.url).href;

export default function MainApp() {
  const app = useMainAppController();
  const { state: configState, refs: configRefs, actions: configActions } = app.config;
  const { state: historyState, actions: historyActions } = app.history;
  const { state: audioState, actions: audioActions } = app.audio;
  const { state: permissionState, actions: permissionActions } = app.permissions;
  const { state: systemState, actions: systemActions } = app.system;
  const { config, correctionSection, dirty, language, page, pending, text } = configState;
  const correctionNavItems = [
    { section: "requirements" as const, label: text.correction.requirementsNav },
    { section: "dictionary" as const, label: text.correction.dictionaryNav },
    { section: "rules" as const, label: text.correction.rulesNav },
    { section: "prompt" as const, label: text.correction.promptNav },
  ];

  return (
    <div className="app-shell">
      <aside className={page === "correction" ? "sidebar correction-active" : "sidebar"}>
        <div className="brand">
          <img className="brand-mark" src={appIconUrl} alt="" aria-hidden="true" />
          <div className="brand-copy">
            <div className="brand-title">BoltScribe</div>
            <div className="brand-subtitle">{text.appSubtitle}</div>
            {systemState.appVersion ? (
              <div className="brand-version">{text.common.version(systemState.appVersion)}</div>
            ) : null}
          </div>
          <button
            className="brand-github-button"
            type="button"
            onClick={() => { void systemActions.openGitHub(); }}
            aria-label={text.nav.openGitHubRepository}
            title={text.nav.openGitHubRepository}
          >
            <GitHubIcon />
          </button>
        </div>
        <nav className="sidebar-nav" aria-label={language === "zh-CN" ? "主导航" : "Main navigation"}>
          <NavButton current={page} page="home" onClick={configActions.requestPageChange} label={text.nav.home} />
          <NavButton current={page} page="history" onClick={configActions.requestPageChange} label={text.nav.history} />
          <NavButton current={page} page="models" onClick={configActions.requestPageChange} label={text.nav.models} />
          <div className="correction-nav-group">
            <div className="correction-nav-primary">
              <NavButton
                current={page}
                page="correction"
                onClick={configActions.requestPageChange}
                label={text.nav.correction}
                ariaExpanded={page === "correction"}
              />
              <label className="nav-toggle" title={text.correction.enabled}>
                <span className="visually-hidden">{text.correction.enabled}</span>
                <input
                  type="checkbox"
                  role="switch"
                  checked={config?.correction.enabled ?? false}
                  disabled={!config || app.view.busy}
                  onChange={(event) => {
                    if (config) {
                      configActions.change({
                        ...config,
                        correction: { ...config.correction, enabled: event.target.checked },
                      });
                    }
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
                    onClick={() => configActions.setCorrectionSection(item.section)}
                  >
                    {item.label}
                  </button>
                ))}
              </nav>
            ) : null}
          </div>
          <NavButton current={page} page="settings" onClick={configActions.requestPageChange} label={text.nav.settings} />
        </nav>
        <div className="sidebar-footer">
          <InputStatsCard stats={historyState.stats} text={text} />
          <button className="ghost-button" onClick={() => { void systemActions.openAppDir(); }}>
            {text.nav.openDataDir}
          </button>
        </div>
      </aside>

      <main className="content">
        <div
          className={page === "correction" ? "content-scroll correction-content" : "content-scroll"}
          ref={configRefs.contentScrollRef}
        >
          {permissionState.needsAccessibility && permissionState.accessibilityGranted === false ? (
            <div className="permission-banner">
              <div>
                <strong>{text.permission.bannerTitle}</strong>
                <span>{text.permission.bannerText}</span>
              </div>
              <div className="permission-actions">
                <button
                  className="secondary small"
                  onClick={() => permissionActions.refreshAccessibility().catch((error) => app.notice.show(String(error)))}
                >
                  {text.permission.recheck}
                </button>
                <button className="secondary small" onClick={permissionActions.openGuide}>{text.home.permissionGuide}</button>
                <button className="secondary small" onClick={permissionActions.openAccessibilityPermission}>{text.permission.openSettings}</button>
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
              status={app.workflow.status}
              busy={app.view.busy}
              history={historyState.recent}
              inputDevicesChecked={audioState.inputDevicesChecked}
              hasInputDevice={audioState.inputDevices.length > 0}
              audioDevicesRefreshing={audioState.refreshing}
              onToggle={app.workflow.actions.toggleRecording}
              onCancel={app.workflow.actions.cancel}
              onOpenPermissionGuide={permissionActions.openGuide}
              onRefreshAudioDevices={() => { void audioActions.refreshDevices().catch((error) => app.notice.show(String(error))); }}
              onRefreshHistory={historyActions.refreshRecent}
              onOpenHistoryPage={app.actions.openHistoryPage}
              onOpenModels={() => configActions.requestPageChange("models")}
              onOpenCorrectionSection={configActions.requestCorrectionSection}
              onCopyHistory={historyActions.copyText}
              onRetryHistory={historyActions.retryRecord}
              onDeleteHistory={historyActions.deleteRecord}
              canRetryHistory={app.view.canMutateHistory}
              canDeleteHistory={app.view.canMutateHistory}
              language={language}
              text={text}
            />
          ) : null}
          {config && page === "history" ? (
            <HistoryRecordsPage
              history={historyState.records}
              pageIndex={historyState.pageIndex}
              pageSize={historyPageSize}
              hasOlder={historyState.hasOlder}
              onRefresh={() => {
                void historyActions.changePage(historyState.pageIndex)
                  .catch((error) => app.notice.show(String(error)));
              }}
              onPreviousPage={() => {
                void historyActions.changePage(Math.max(0, historyState.pageIndex - 1))
                  .catch((error) => app.notice.show(String(error)));
              }}
              onNextPage={() => {
                void historyActions.changePage(historyState.pageIndex + 1)
                  .catch((error) => app.notice.show(String(error)));
              }}
              onCopyHistory={historyActions.copyText}
              onRetryHistory={historyActions.retryRecord}
              onDeleteHistory={historyActions.deleteRecord}
              canRetryHistory={app.view.canMutateHistory}
              canDeleteHistory={app.view.canMutateHistory}
              text={text}
            />
          ) : null}
          {config && page === "models" ? (
            <ModelsPage
              config={config}
              onChange={configActions.change}
              onSaveConfig={configActions.save}
              onNotice={app.notice.show}
              canSave={app.view.canSave}
              text={text}
            />
          ) : null}
          {config && page === "correction" ? (
            <CorrectionPage
              config={config}
              onChange={configActions.change}
              section={correctionSection}
              text={text}
            />
          ) : null}
          {config && page === "settings" ? (
            <SettingsPage
              config={config}
              audioDevices={audioState.inputDevices}
              audioOutputDevices={audioState.outputDevices}
              dataDir={systemState.dataDir}
              audioDevicesRefreshing={audioState.refreshing}
              audioServiceRestarting={audioState.serviceRestarting}
              onChange={configActions.change}
              onExportConfig={() => { void configActions.exportCurrent(); }}
              onImportConfig={(file) => { void configActions.importFile(file); }}
              onOpenDataDir={() => { void systemActions.openAppDir(); }}
              onChooseDataDir={() => { void systemActions.changeDataDir(); }}
              onResetDataDir={() => { void systemActions.resetDataDir(); }}
              onCleanupRecordingFiles={historyActions.cleanupRecordings}
              onPreviewRecordingCleanup={historyActions.previewCleanup}
              onRefreshAudioDevices={() => { void audioActions.refreshDevices().catch((error) => app.notice.show(String(error))); }}
              onRestartAudioService={() => { void audioActions.restartService(); }}
              vadTestStatus={audioState.vadStatus}
              onStartVadTest={() => { void audioActions.startVadTest(); }}
              onUpdateVadTestSettings={(noiseMarginDb, confirmationMs, noiseWindowMs) => {
                void audioActions.updateVadSettings(noiseMarginDb, confirmationMs, noiseWindowMs);
              }}
              onStopVadTest={() => { void audioActions.stopVadTest(); }}
              inputMonitoringGranted={permissionState.inputMonitoringGranted}
              inputMonitoringPermission={permissionState.inputMonitoringPermission}
              onRefreshInputMonitoring={() => { void permissionActions.refreshInputMonitoring().catch((error) => app.notice.show(String(error))); }}
              onRequestInputMonitoring={() => { void permissionActions.requestInputMonitoring(); }}
              onApplyFnTrigger={(enabled) => { void permissionActions.applyCurrentFnTrigger(enabled).catch((error) => app.notice.show(String(error))); }}
              importReport={configState.importReport}
              canSave={app.view.canSave}
              canChangeDataDir={app.view.canChangeDataDir}
              canCleanupRecordings={app.view.canChangeDataDir}
              canRestartAudioService={app.view.canMutateHistory}
              text={text}
            />
          ) : null}
        </div>
        {app.view.showSaveBar ? (
          <div className={dirty ? "config-save-bar dirty" : "config-save-bar"}>
            <div className="config-save-status" role="status" aria-live="polite">
              <span className="config-save-dot" aria-hidden="true" />
              <span>{dirty ? text.unsaved.statusPending : text.unsaved.statusSaved}</span>
            </div>
            <button
              className="primary small"
              type="button"
              disabled={!app.view.canSaveChanges}
              onClick={configActions.saveCurrent}
            >
              {configState.saving ? text.common.saving : text.common.save}
            </button>
          </div>
        ) : null}
      </main>
      {pending ? (
        <div className="modal-backdrop" role="presentation">
          <section
            ref={configRefs.dialogRef}
            className="unsaved-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="unsaved-title"
            aria-describedby="unsaved-message"
            onKeyDown={configActions.handleDialogKeyDown}
          >
            <div>
              <h1 id="unsaved-title">{text.unsaved.title}</h1>
              <p id="unsaved-message">{text.unsaved.message}</p>
            </div>
            <div className="modal-footer">
              <button ref={configRefs.cancelButtonRef} className="secondary small" type="button" onClick={configActions.dismissPending} disabled={app.view.busy}>{text.unsaved.cancel}</button>
              <button className="secondary small" type="button" onClick={() => { void configActions.discardPending(); }} disabled={app.view.busy}>{text.unsaved.discard}</button>
              <button className="primary small" type="button" onClick={() => { void configActions.savePending(); }} disabled={app.view.busy || !dirty}>{text.unsaved.save}</button>
            </div>
          </section>
        </div>
      ) : null}
      {permissionState.guideVisible ? (
        <PermissionGuide
          accessibilityGranted={permissionState.accessibilityGranted}
          requiresAccessibility={permissionState.needsAccessibility}
          microphonePermission={permissionState.microphonePermission}
          onClose={permissionActions.closeGuide}
          onRefreshAccessibility={() => permissionActions.refreshAccessibility().catch((error) => app.notice.show(String(error)))}
          onOpenAccessibility={permissionActions.openAccessibilityPermission}
          onRequestMicrophone={permissionActions.requestMicrophone}
          text={text}
        />
      ) : null}
      {app.notice.current ? (
        <div className="toast" key={app.notice.current.id} role="status" aria-live="polite" aria-atomic="true">
          <span>{app.notice.current.message}</span>
          <button className="toast-close" type="button" onClick={() => app.notice.dismiss(app.notice.current!.id)} aria-label={text.notices.closeNotice}>×</button>
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
