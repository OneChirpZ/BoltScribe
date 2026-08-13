import { useEffect, useRef, useState } from "react";
import type { AppConfig, AudioLevelSample, WorkflowStatus } from "../types";
import RecordingOverlay from "../components/RecordingOverlay";
import { appLanguage, translations } from "../domain/i18n";
import { defaultRecordingOverlayScale, mergeRecordingOverlayPosition, recordingOverlayBaseWidth } from "../domain/overlay";
import { emptyStatus, latestWorkflowStatus, subscribeToWorkflowStatus } from "../domain/workflow";
import { appApi } from "./api";

const {
  config: { listenConfigUpdated, loadConfig },
  overlay: { listenRecordingOverlayPositionUpdated, startRecordingOverlayDrag },
  workflow: { cancelCurrentWorkflow, getStatus, listenWorkflowStatus },
  audio: { listenAudioLevel },
} = appApi;

export default function OverlayApp() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);
  const statusRef = useRef<WorkflowStatus>(emptyStatus);
  const [audioLevelSample, setAudioLevelSample] = useState<AudioLevelSample & { sequence: number }>({
    level: 0,
    recording_revision: 0,
    sequence: 0,
  });

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
      onStatus: (nextStatus) => setStatus((current) => {
        const latest = latestWorkflowStatus(current, nextStatus);
        statusRef.current = latest;
        return latest;
      }),
    });
    const unlistenAudioLevel = listenAudioLevel((sample) => {
      const currentStatus = statusRef.current;
      if (currentStatus.mode !== "recording" || sample.recording_revision !== currentStatus.revision) {
        return;
      }
      setAudioLevelSample((current) => ({ ...sample, sequence: current.sequence + 1 }));
    });
    const unlistenConfig = listenConfigUpdated(setConfig);
    const unlistenPosition = listenRecordingOverlayPositionUpdated((update) => {
      setConfig((current) => current ? mergeRecordingOverlayPosition(current, update) : current);
    });

    return () => {
      disposed = true;
      document.documentElement.classList.remove("overlay-window");
      document.body.classList.remove("overlay-window");
      stopStatusSubscription();
      unlistenAudioLevel.then((fn) => fn());
      unlistenConfig.then((fn) => fn());
      unlistenPosition.then((fn) => fn());
    };
  }, []);

  function beginDragSession() {
    try {
      void startRecordingOverlayDrag().catch(() => undefined);
    } catch {
      // Keep the non-activating overlay quiet if native dragging is unavailable.
    }
  }

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
        audioLevelRevision={audioLevelSample.recording_revision}
        audioLevelSequence={audioLevelSample.sequence}
        baseWidth={recordingOverlayBaseWidth(language)}
        scale={config?.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
        onCancel={cancelWorkflow}
        onDragStart={beginDragSession}
        text={text}
      />
    </div>
  );
}
