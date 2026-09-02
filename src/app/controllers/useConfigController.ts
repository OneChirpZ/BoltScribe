import {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import type { AppConfig, ConfigImportReport } from "../../types";
import { appLanguage, translations } from "../../domain/i18n";
import type { CorrectionSection, Page } from "../../domain/navigation";
import { mergeRecordingOverlayPosition } from "../../domain/overlay";
import { supportsFnLongPressTrigger } from "../../domain/platform";
import { applyThemePreference } from "../../domain/theme";
import type { AppApi } from "../api";
import type { SetBusy, SetNotice } from "./controllerTypes";

export type PendingConfigAction =
  | {
      kind: "page";
      page: Page;
      correctionSection?: CorrectionSection;
      resetHistoryPage?: boolean;
    }
  | { kind: "close" };

interface ConfigState {
  draft: AppConfig | null;
  saved: AppConfig | null;
  saving: boolean;
  pending: PendingConfigAction | null;
  importReport: ConfigImportReport | null;
}

type ConfigAction =
  | { type: "loaded"; config: AppConfig }
  | { type: "changed"; config: AppConfig }
  | { type: "saved-only"; config: AppConfig }
  | { type: "overlay-position-updated"; draft: AppConfig | null; saved: AppConfig | null }
  | { type: "saving"; value: boolean }
  | { type: "save-succeeded"; config: AppConfig; updateDraft: boolean }
  | { type: "pending"; action: PendingConfigAction | null }
  | { type: "imported"; config: AppConfig; report: ConfigImportReport };

const initialState: ConfigState = {
  draft: null,
  saved: null,
  saving: false,
  pending: null,
  importReport: null,
};

function reducer(state: ConfigState, action: ConfigAction): ConfigState {
  switch (action.type) {
    case "loaded":
      return { ...state, draft: action.config, saved: action.config };
    case "changed":
      return { ...state, draft: action.config };
    case "saved-only":
      return { ...state, saved: action.config };
    case "overlay-position-updated":
      return { ...state, draft: action.draft, saved: action.saved };
    case "saving":
      return { ...state, saving: action.value };
    case "save-succeeded":
      return {
        ...state,
        saved: action.config,
        draft: action.updateDraft ? action.config : state.draft,
      };
    case "pending":
      return { ...state, pending: action.action };
    case "imported":
      return {
        ...state,
        draft: action.config,
        saved: action.config,
        importReport: action.report,
      };
  }
}

export function useConfigController({
  api,
  busy,
  setBusy,
  setNotice,
}: {
  api: AppApi;
  busy: boolean;
  setBusy: SetBusy;
  setNotice: SetNotice;
}) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const [page, setPage] = useState<Page>("home");
  const [correctionSection, setCorrectionSection] = useState<CorrectionSection>("requirements");
  const draftRef = useRef<AppConfig | null>(null);
  const savedRef = useRef<AppConfig | null>(null);
  const savingRef = useRef(false);
  const pageRef = useRef(page);
  const contentScrollRef = useRef<HTMLDivElement | null>(null);
  const completedPendingRef = useRef<PendingConfigAction | null>(null);
  const dialogRef = useRef<HTMLElement | null>(null);
  const cancelButtonRef = useRef<HTMLButtonElement | null>(null);
  const focusBeforeDialogRef = useRef<HTMLElement | null>(null);

  const language = appLanguage(state.draft);
  const text = translations[language];
  const dirty = isConfigDirty(state.draft, state.saved);

  const applyLoaded = useCallback((config: AppConfig) => {
    draftRef.current = config;
    savedRef.current = config;
    dispatch({ type: "loaded", config });
  }, []);

  const change = useCallback((config: AppConfig) => {
    draftRef.current = config;
    dispatch({ type: "changed", config });
  }, []);

  const loadInitial = useCallback(async () => {
    const config = await api.config.loadConfig();
    applyLoaded(config);
    return config;
  }, [api, applyLoaded]);

  const persist = useCallback(async (nextConfig: AppConfig, successMessage?: string) => {
    savingRef.current = true;
    setBusy(true);
    dispatch({ type: "saving", value: true });
    try {
      const expectedOverlayPosition = savedRef.current
        ? {
            recording_overlay_offset_x: savedRef.current.ui.recording_overlay_offset_x,
            recording_overlay_offset_y: savedRef.current.ui.recording_overlay_offset_y,
          }
        : null;
      const result = await api.config.saveConfig(nextConfig, expectedOverlayPosition);
      const updateDraft = !isConfigDirty(draftRef.current, nextConfig);
      savedRef.current = result.config;
      if (updateDraft) {
        draftRef.current = result.config;
      }
      dispatch({ type: "save-succeeded", config: result.config, updateDraft });
      const warning = result.warnings.map((item) => item.detail).join("；");
      setNotice(warning || successMessage || text.notices.configSaved);
      return result.config;
    } catch (error) {
      setNotice(String(error));
      return null;
    } finally {
      savingRef.current = false;
      dispatch({ type: "saving", value: false });
      setBusy(false);
    }
  }, [api, setBusy, setNotice, text.notices]);

  const save = useCallback(async (nextConfig: AppConfig, successMessage?: string) => {
    await persist(nextConfig, successMessage);
  }, [persist]);

  const completePending = useCallback((action: PendingConfigAction) => {
    if (action.kind === "page") {
      completedPendingRef.current = action;
      if (action.correctionSection) {
        setCorrectionSection(action.correctionSection);
      }
      setPage(action.page);
      return;
    }
    api.system.closeMainWindow().catch((error) => setNotice(String(error)));
  }, [api, setNotice]);

  const requestPageChange = useCallback((nextPage: Page, resetHistoryPage = false) => {
    if (nextPage === page) {
      return;
    }
    if (savingRef.current) {
      return;
    }
    if (isConfigDirty(draftRef.current, savedRef.current)) {
      const action: PendingConfigAction = resetHistoryPage
        ? { kind: "page", page: nextPage, resetHistoryPage: true }
        : { kind: "page", page: nextPage };
      dispatch({
        type: "pending",
        action,
      });
      return;
    }
    setPage(nextPage);
  }, [page]);

  const requestCorrectionSection = useCallback((nextSection: CorrectionSection) => {
    if (savingRef.current) {
      return;
    }
    if (page === "correction") {
      setCorrectionSection(nextSection);
      return;
    }
    if (isConfigDirty(draftRef.current, savedRef.current)) {
      dispatch({
        type: "pending",
        action: { kind: "page", page: "correction", correctionSection: nextSection },
      });
      return;
    }
    setCorrectionSection(nextSection);
    setPage("correction");
  }, [page]);

  const requestWindowClose = useCallback(() => {
    if (savingRef.current) {
      return;
    }
    if (isConfigDirty(draftRef.current, savedRef.current)) {
      dispatch({ type: "pending", action: { kind: "close" } });
      return;
    }
    api.system.closeMainWindow().catch((error) => setNotice(String(error)));
  }, [api, setNotice]);

  const savePending = useCallback(async () => {
    const action = state.pending;
    const nextConfig = draftRef.current;
    if (!action || !nextConfig) {
      return;
    }
    if (!await persist(nextConfig)) {
      return;
    }
    dispatch({ type: "pending", action: null });
    completePending(action);
  }, [completePending, persist, state.pending]);

  const discardPending = useCallback(async () => {
    const action = state.pending;
    if (!action) {
      return;
    }
    const persisted = savedRef.current;
    if (persisted) {
      applyLoaded(persisted);
      if (supportsFnLongPressTrigger()) {
        setBusy(true);
        try {
          await api.system.applyFnTrigger(
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
    dispatch({ type: "pending", action: null });
    completePending(action);
  }, [api, applyLoaded, completePending, setBusy, setNotice, state.pending]);

  const exportCurrent = useCallback(async () => {
    const current = draftRef.current;
    if (!current) {
      return;
    }
    setBusy(true);
    try {
      const path = await api.config.exportConfig(current);
      setNotice(text.notices.configExported(path));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, setBusy, setNotice, text.notices]);

  const importFile = useCallback(async (file: File) => {
    setBusy(true);
    try {
      const result = await api.config.importConfig(await file.text());
      draftRef.current = result.config;
      savedRef.current = result.config;
      dispatch({ type: "imported", config: result.config, report: result.report });
      const warning = result.warnings.map((item) => item.detail).join("；");
      setNotice(warning || text.notices.configImported);
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, setBusy, setNotice, text.notices]);

  const saveCurrent = useCallback(() => {
    const current = draftRef.current;
    if (current) {
      void save(current);
    }
  }, [save]);

  const dismissPending = useCallback(() => {
    dispatch({ type: "pending", action: null });
  }, []);

  const handleDialogKeyDown = useCallback((event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      dismissPending();
      return;
    }
    if (event.key !== "Tab") {
      return;
    }
    const buttons = Array.from(dialogRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
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
  }, [busy, dismissPending]);

  useEffect(() => {
    const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => applyThemePreference(state.draft?.ui.theme, systemTheme.matches);
    applyTheme();
    if (state.draft?.ui.theme === "light" || state.draft?.ui.theme === "dark") {
      return;
    }
    systemTheme.addEventListener("change", applyTheme);
    window.addEventListener("focus", applyTheme);
    document.addEventListener("visibilitychange", applyTheme);
    return () => {
      systemTheme.removeEventListener("change", applyTheme);
      window.removeEventListener("focus", applyTheme);
      document.removeEventListener("visibilitychange", applyTheme);
    };
  }, [state.draft?.ui.theme]);

  useEffect(() => {
    pageRef.current = page;
    if (contentScrollRef.current) {
      contentScrollRef.current.scrollTop = 0;
    }
  }, [page]);

  useEffect(() => {
    if (!state.pending) {
      const previousFocus = focusBeforeDialogRef.current;
      focusBeforeDialogRef.current = null;
      previousFocus?.focus();
      return;
    }
    if (!focusBeforeDialogRef.current && document.activeElement instanceof HTMLElement) {
      focusBeforeDialogRef.current = document.activeElement;
    }
    if (busy) {
      return;
    }
    const frame = window.requestAnimationFrame(() => cancelButtonRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [state.pending, busy]);

  useEffect(() => {
    const unlistenConfig = api.config.listenConfigUpdated((updatedConfig) => {
      if (isConfigDirty(draftRef.current, savedRef.current)) {
        savedRef.current = updatedConfig;
        dispatch({ type: "saved-only", config: updatedConfig });
        return;
      }
      applyLoaded(updatedConfig);
    });
    const unlistenClose = api.config.listenConfigCloseRequested(requestWindowClose);
    const unlistenOverlayPosition = api.overlay.listenRecordingOverlayPositionUpdated((update) => {
      const nextDraft = draftRef.current
        ? mergeRecordingOverlayPosition(draftRef.current, update)
        : null;
      const nextSaved = savedRef.current
        ? mergeRecordingOverlayPosition(savedRef.current, update)
        : null;
      draftRef.current = nextDraft;
      savedRef.current = nextSaved;
      dispatch({ type: "overlay-position-updated", draft: nextDraft, saved: nextSaved });
    });
    return () => {
      unlistenConfig.then((stop) => stop());
      unlistenClose.then((stop) => stop());
      unlistenOverlayPosition.then((stop) => stop());
    };
  }, [api, applyLoaded, requestWindowClose]);

  return {
    state: {
      config: state.draft,
      savedConfig: state.saved,
      saving: state.saving,
      pending: state.pending,
      importReport: state.importReport,
      dirty,
      language,
      text,
      page,
      correctionSection,
    },
    refs: {
      draftRef,
      pageRef,
      contentScrollRef,
      completedPendingRef,
      dialogRef,
      cancelButtonRef,
    },
    actions: {
      applyLoaded,
      change,
      discardPending,
      dismissPending,
      exportCurrent,
      handleDialogKeyDown,
      importFile,
      loadInitial,
      requestCorrectionSection,
      requestPageChange,
      requestWindowClose,
      save,
      saveCurrent,
      savePending,
      setCorrectionSection,
      setPage,
    },
  };
}

function isConfigDirty(config: AppConfig | null, savedConfig: AppConfig | null) {
  if (!config || !savedConfig) {
    return false;
  }
  return JSON.stringify(config) !== JSON.stringify(savedConfig);
}
