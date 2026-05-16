import { useEffect, useState } from "react";
import type { AppConfig, WorkflowStatus } from "../types";
import RecordingOverlay from "../components/RecordingOverlay";
import { appLanguage, translations } from "../domain/i18n";
import { defaultRecordingOverlayScale } from "../domain/overlay";
import { emptyStatus } from "../domain/workflow";
import { cancelCurrentWorkflow, getStatus, listenConfigUpdated, listenWorkflowStatus, loadConfig } from "./tauriApi";

export default function OverlayApp() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);

  useEffect(() => {
    document.documentElement.classList.add("overlay-window");
    document.body.classList.add("overlay-window");

    Promise.all([
      loadConfig(),
      getStatus(),
    ]).then(([loadedConfig, loadedStatus]) => {
      setConfig(loadedConfig);
      setStatus(loadedStatus);
    }).catch(() => undefined);

    const unlistenStatus = listenWorkflowStatus(setStatus);
    const unlistenConfig = listenConfigUpdated(setConfig);

    return () => {
      document.documentElement.classList.remove("overlay-window");
      document.body.classList.remove("overlay-window");
      unlistenStatus.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
    };
  }, []);

  async function cancelWorkflow() {
    try {
      const nextStatus = await cancelCurrentWorkflow();
      setStatus(nextStatus);
    } catch {
      // The overlay intentionally stays quiet; the main window will surface errors.
    }
  }

  const text = translations[appLanguage(config)];

  return (
    <div className="overlay-shell">
      <RecordingOverlay
        status={status}
        scale={config?.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
        onCancel={cancelWorkflow}
        text={text}
      />
    </div>
  );
}
