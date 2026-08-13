import { useCallback, useEffect, useState, type MutableRefObject } from "react";
import type { AppConfig } from "../../types";
import type { PermissionRequestState } from "../../domain/permissions";
import { requiresAccessibilityPermission } from "../../domain/platform";
import type { AppApi } from "../api";
import type { AppText, SetNotice } from "./controllerTypes";

export function usePermissionsController({
  api,
  config,
  configRef,
  setNotice,
  text,
}: {
  api: AppApi;
  config: AppConfig | null;
  configRef: MutableRefObject<AppConfig | null>;
  setNotice: SetNotice;
  text: AppText;
}) {
  const needsAccessibility = requiresAccessibilityPermission();
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);
  const [inputMonitoringGranted, setInputMonitoringGranted] = useState<boolean | null>(null);
  const [inputMonitoringPermission, setInputMonitoringPermission] = useState<PermissionRequestState>("unknown");
  const [microphonePermission, setMicrophonePermission] = useState<PermissionRequestState>("unknown");
  const [guideVisible, setGuideVisible] = useState(false);

  const refreshAccessibility = useCallback(async () => {
    const granted = await api.permissions.accessibilityPermissionGranted();
    setAccessibilityGranted(granted);
    return granted;
  }, [api]);

  const initializeAccessibility = useCallback(async () => {
    const granted = await refreshAccessibility();
    if (needsAccessibility && !granted) {
      setGuideVisible(true);
    }
    return granted;
  }, [needsAccessibility, refreshAccessibility]);

  const applyCurrentFnTrigger = useCallback(async (
    enabled = configRef.current?.system.fn_long_press_enabled ?? false,
  ) => {
    const currentConfig = configRef.current;
    if (!currentConfig) {
      return false;
    }
    await api.system.applyFnTrigger(enabled, currentConfig.system.fn_long_press_duration_ms ?? 200);
    return true;
  }, [api, configRef]);

  const refreshInputMonitoring = useCallback(async () => {
    setInputMonitoringPermission("checking");
    const granted = await api.permissions.inputMonitoringPermissionGranted();
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
  }, [api, applyCurrentFnTrigger, configRef, setNotice, text.permission]);

  const openAccessibilityPermission = useCallback(async () => {
    try {
      const granted = await api.permissions.requestAccessibilityPermission();
      const latest = granted || await refreshAccessibility();
      setAccessibilityGranted(latest);
      if (!latest) {
        await api.permissions.openAccessibilitySettings();
        setNotice(text.permission.accessibilityNotice);
      } else {
        setNotice(text.permission.accessibilityGranted);
      }
    } catch (error) {
      setNotice(String(error));
    }
  }, [api, refreshAccessibility, setNotice, text.permission]);

  const requestInputMonitoring = useCallback(async () => {
    setInputMonitoringPermission("checking");
    try {
      const granted = await api.permissions.requestInputMonitoringPermission();
      const latest = granted || await refreshInputMonitoring();
      if (!latest) {
        setInputMonitoringGranted(false);
        setInputMonitoringPermission("denied");
        await api.permissions.openInputMonitoringSettings();
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
          await api.permissions.openInputMonitoringSettings();
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
  }, [api, applyCurrentFnTrigger, configRef, refreshInputMonitoring, setNotice, text.permission]);

  const requestMicrophone = useCallback(async () => {
    setMicrophonePermission("checking");
    try {
      const granted = await api.permissions.requestMicrophonePermission();
      setMicrophonePermission(granted ? "granted" : "denied");
      setNotice(granted ? text.permission.microphoneGranted : text.permission.microphoneDenied);
    } catch (error) {
      setMicrophonePermission("denied");
      setNotice(String(error));
    }
  }, [api, setNotice, text.permission]);

  useEffect(() => {
    if (!needsAccessibility || accessibilityGranted !== false) {
      return;
    }
    const timer = window.setInterval(() => {
      refreshAccessibility().catch(() => undefined);
    }, 2000);
    return () => window.clearInterval(timer);
  }, [accessibilityGranted, needsAccessibility, refreshAccessibility]);

  useEffect(() => {
    if (!config?.system.fn_long_press_enabled) {
      setInputMonitoringGranted(null);
      setInputMonitoringPermission("unknown");
      return;
    }
    if (inputMonitoringPermission === "unknown") {
      refreshInputMonitoring().catch((error) => setNotice(String(error)));
    }
  }, [config?.system.fn_long_press_enabled, inputMonitoringPermission, refreshInputMonitoring, setNotice]);

  return {
    state: {
      accessibilityGranted,
      guideVisible,
      inputMonitoringGranted,
      inputMonitoringPermission,
      microphonePermission,
      needsAccessibility,
    },
    actions: {
      applyCurrentFnTrigger,
      closeGuide: () => setGuideVisible(false),
      initializeAccessibility,
      openAccessibilityPermission,
      openGuide: () => setGuideVisible(true),
      refreshAccessibility,
      refreshInputMonitoring,
      requestInputMonitoring,
      requestMicrophone,
    },
  };
}
