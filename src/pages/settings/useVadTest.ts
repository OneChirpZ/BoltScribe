import { useEffect, useRef, useState } from "react";
import { appendAudioLevelSample, createAudioLevelHistory, dbfsToDisplayLevel } from "../../domain/audioLevel";
import type { TextBundle } from "../../domain/i18n";
import type { VadTestStatus } from "../../types";

export function useVadTest(status: VadTestStatus, onStop: () => void, text: TextBundle) {
  const [levelHistory, setLevelHistory] = useState(createAudioLevelHistory);
  const stopRef = useRef(onStop);
  const statusRef = useRef(status);
  stopRef.current = onStop;
  statusRef.current = status;

  useEffect(() => () => {
    if (statusRef.current.mode !== "idle") {
      stopRef.current();
    }
  }, []);

  useEffect(() => {
    if (status.mode !== "listening" && status.mode !== "voice") {
      setLevelHistory(createAudioLevelHistory());
      return;
    }
    setLevelHistory((history) => appendAudioLevelSample(history, dbfsToDisplayLevel(status.level)));
  }, [status.level, status.mode, status.revision]);

  const label = status.mode === "voice"
    ? text.settings.vadVoiceDetected
    : status.mode === "listening"
      ? !status.noise_calibrated
        ? text.settings.vadCalibrating
        : status.voice_active
          ? text.settings.vadConfirmingVoice
          : status.raw_voice_active
            ? text.settings.vadFilteringNoise
            : text.settings.vadWaiting
      : status.mode === "timed_out"
        ? text.settings.vadTestTimeout
        : status.mode === "error"
          ? text.settings.vadTestError
          : text.settings.vadReady;

  const visualState = status.mode === "voice"
    ? "voice"
    : status.voice_active
      ? "voice-pending"
      : status.raw_voice_active
        ? "filtering"
        : status.mode === "listening"
          ? "listening"
          : status.mode === "timed_out"
            ? "timed-out"
            : status.mode === "error"
              ? "error"
              : "idle";

  return {
    running: status.mode !== "idle",
    label,
    levelHistory,
    visualState,
  };
}
