import { useEffect, useState } from "react";
import type { AppConfig, WorkflowStatus } from "../types";
import RecordingOverlay from "../components/RecordingOverlay";
import { appLanguage, translations } from "../domain/i18n";
import { defaultRecordingOverlayScale, recordingOverlayBaseWidth } from "../domain/overlay";
import { emptyStatus, latestWorkflowStatus, subscribeToWorkflowStatus } from "../domain/workflow";
import { cancelCurrentWorkflow, getStatus, listenAudioLevel, listenConfigUpdated, listenWorkflowStatus, loadConfig } from "./tauriApi";

export default function OverlayApp() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);
  const [audioLevelSample, setAudioLevelSample] = useState({ level: 0, sequence: 0 });

  useEffect(() => {
    document.documentElement.classList.add("overlay-window");
    document.body.classList.add("overlay-window");

    let disposed = false;
    loadConfig().then((loadedConfig) => {
      if (!disposed) {
        setConfig(loadedConfig);
      }
    }).catch(() => undefined);

    const stopStatusSubscription = subscribeToWorkflowStatus({
      listen: listenWorkflowStatus,
      getSnapshot: getStatus,
      onStatus: (nextStatus) => setStatus((current) => latestWorkflowStatus(current, nextStatus)),
    });
    const unlistenAudioLevel = listenAudioLevel((level) => {
      setAudioLevelSample((sample) => ({ level, sequence: sample.sequence + 1 }));
    });
    const unlistenConfig = listenConfigUpdated(setConfig);

    return () => {
      disposed = true;
      document.documentElement.classList.remove("overlay-window");
      document.body.classList.remove("overlay-window");
      stopStatusSubscription();
      unlistenAudioLevel.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
    };
  }, []);

  async function cancelWorkflow() {
    try {
      const nextStatus = await cancelCurrentWorkflow();
      setStatus((current) => latestWorkflowStatus(current, nextStatus));
    } catch {
      // The overlay intentionally stays quiet; the main window will surface errors.
    }
  }

  const language = appLanguage(config);
  const text = translations[language];

  return (
    <div className="overlay-shell">
      <RecordingOverlay
        status={status}
        audioLevel={audioLevelSample.level}
        audioLevelSequence={audioLevelSample.sequence}
        baseWidth={recordingOverlayBaseWidth(language)}
        scale={config?.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
        onCancel={cancelWorkflow}
        text={text}
      />
    </div>
  );
}
