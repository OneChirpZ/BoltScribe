import { useCallback, useEffect, useRef, useState } from "react";
import type { AppConfig, AudioInputDevice, AudioOutputDevice, VadTestStatus, WorkflowStatus } from "../../types";
import type { AppApi } from "../api";
import type { AppText, SetBusy, SetNotice } from "./controllerTypes";

const initialVadTestStatus: VadTestStatus = {
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
  noise_window_ms: 800,
  revision: 0,
  error: null,
};

export function useAudioSettingsController({
  api,
  busy,
  config,
  setBusy,
  setNotice,
  text,
  workflowStatus,
}: {
  api: AppApi;
  busy: boolean;
  config: AppConfig | null;
  setBusy: SetBusy;
  setNotice: SetNotice;
  text: AppText;
  workflowStatus: WorkflowStatus;
}) {
  const [inputDevices, setInputDevices] = useState<AudioInputDevice[]>([]);
  const [outputDevices, setOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [inputDevicesChecked, setInputDevicesChecked] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [serviceRestarting, setServiceRestarting] = useState(false);
  const [vadStatus, setVadStatus] = useState(initialVadTestStatus);
  const vadSettingsRequestRef = useRef(0);
  const vadTestStartingRef = useRef(false);
  const vadTestStartPromiseRef = useRef<Promise<void> | null>(null);

  const acceptVadStatus = useCallback((nextStatus: VadTestStatus) => {
    setVadStatus((current) => nextStatus.revision > current.revision ? nextStatus : current);
  }, []);

  const refreshDevices = useCallback(async () => {
    setRefreshing(true);
    try {
      const [inputResult, outputResult] = await Promise.allSettled([
        api.audio.loadAudioInputDevices(),
        api.audio.loadAudioOutputDevices(),
      ]);
      if (inputResult.status === "fulfilled") {
        setInputDevices(inputResult.value);
        setInputDevicesChecked(true);
      }
      if (outputResult.status === "fulfilled") {
        setOutputDevices(outputResult.value);
      }
      const errors = [inputResult, outputResult]
        .filter((result): result is PromiseRejectedResult => result.status === "rejected")
        .map((result) => String(result.reason));
      if (errors.length > 0) {
        throw new Error(errors.join("; "));
      }
    } finally {
      setRefreshing(false);
    }
  }, [api]);

  const restartService = useCallback(async () => {
    setServiceRestarting(true);
    setBusy(true);
    try {
      const restarted = await api.audio.restartSystemAudioService();
      if (!restarted) {
        setNotice(text.notices.audioServiceRestartCancelled);
        return;
      }
      try {
        await refreshDevices();
        setNotice(text.notices.audioServiceRestarted);
      } catch (error) {
        setNotice(text.notices.audioServiceRestartedRefreshFailed(String(error)));
      }
    } catch (error) {
      setNotice(text.notices.audioServiceRestartFailed(String(error)));
    } finally {
      setServiceRestarting(false);
      setBusy(false);
    }
  }, [api, refreshDevices, setBusy, setNotice, text.notices]);

  const startVadTest = useCallback(() => {
    if (!config || busy || workflowStatus.mode !== "idle" || vadTestStartingRef.current) {
      return Promise.resolve();
    }
    vadTestStartingRef.current = true;
    const request = ++vadSettingsRequestRef.current;
    const startPromise = (async () => {
      try {
        const nextStatus = await api.audio.startVadTest(config.audio);
        if (request === vadSettingsRequestRef.current) {
          acceptVadStatus(nextStatus);
        }
      } catch (error) {
        if (request === vadSettingsRequestRef.current) {
          setVadStatus((current) => ({
            ...current,
            mode: "error",
            error: String(error),
            revision: current.revision + 1,
          }));
          setNotice(String(error));
        }
      } finally {
        vadTestStartingRef.current = false;
      }
    })();
    vadTestStartPromiseRef.current = startPromise;
    void startPromise.then(() => {
      if (vadTestStartPromiseRef.current === startPromise) {
        vadTestStartPromiseRef.current = null;
      }
    });
    return startPromise;
  }, [acceptVadStatus, api, busy, config, setNotice, workflowStatus.mode]);

  const updateVadSettings = useCallback(async (
    noiseMarginDb: number,
    confirmationMs: number,
    noiseWindowMs: number,
  ) => {
    const request = ++vadSettingsRequestRef.current;
    try {
      const nextStatus = await api.audio.updateVadTestSettings(noiseMarginDb, confirmationMs, noiseWindowMs);
      if (request === vadSettingsRequestRef.current) {
        acceptVadStatus(nextStatus);
      }
    } catch (error) {
      if (request === vadSettingsRequestRef.current) {
        setNotice(String(error));
      }
    }
  }, [acceptVadStatus, api, setNotice]);

  const stopVadTest = useCallback(async () => {
    vadSettingsRequestRef.current += 1;
    await vadTestStartPromiseRef.current;
    try {
      acceptVadStatus(await api.audio.stopVadTest());
    } catch (error) {
      setNotice(String(error));
    }
  }, [acceptVadStatus, api, setNotice]);

  useEffect(() => {
    const unlisten = api.audio.listenVadTestStatus(acceptVadStatus);
    api.audio.getVadTestStatus().then(acceptVadStatus).catch(() => undefined);
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [acceptVadStatus, api]);

  useEffect(() => {
    refreshDevices().catch((error) => setNotice(String(error)));
  }, [refreshDevices, setNotice]);

  return {
    state: {
      inputDevices,
      outputDevices,
      inputDevicesChecked,
      refreshing,
      serviceRestarting,
      vadStatus,
    },
    actions: { refreshDevices, restartService, startVadTest, stopVadTest, updateVadSettings },
  };
}
