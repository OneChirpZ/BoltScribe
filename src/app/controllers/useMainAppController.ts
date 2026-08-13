import { useCallback, useEffect, useState } from "react";
import { appApi } from "../api";
import { useAudioSettingsController } from "./useAudioSettingsController";
import { useConfigController } from "./useConfigController";
import { useHistoryController } from "./useHistoryController";
import { useNoticeController } from "./useNoticeController";
import { usePermissionsController } from "./usePermissionsController";
import { useSystemSettingsController } from "./useSystemSettingsController";
import { useWorkflowController } from "./useWorkflowController";

export function useMainAppController() {
  const notice = useNoticeController();
  const [busy, setBusy] = useState(false);
  const workflow = useWorkflowController({ api: appApi, setBusy, setNotice: notice.show });
  const config = useConfigController({ api: appApi, busy, setBusy, setNotice: notice.show });
  const history = useHistoryController({
    api: appApi,
    page: config.state.page,
    pageRef: config.refs.pageRef,
    setBusy,
    setNotice: notice.show,
    text: config.state.text,
  });
  const audio = useAudioSettingsController({
    api: appApi,
    busy,
    config: config.state.config,
    setBusy,
    setNotice: notice.show,
    text: config.state.text,
    workflowStatus: workflow.status,
  });
  const permissions = usePermissionsController({
    api: appApi,
    config: config.state.config,
    configRef: config.refs.draftRef,
    setNotice: notice.show,
    text: config.state.text,
  });
  const system = useSystemSettingsController({
    api: appApi,
    refreshHistoryAfterDirectoryChange: history.actions.refreshAfterDirectoryChange,
    setBusy,
    setNotice: notice.show,
    text: config.state.text,
  });

  const openHistoryPage = useCallback(() => {
    if (config.state.saving) {
      return;
    }
    if (config.state.page === "history") {
      void history.actions.loadPage(0).catch((error) => notice.show(String(error)));
      return;
    }
    if (!config.state.dirty) {
      history.refs.pageIndexRef.current = 0;
    }
    config.actions.requestPageChange("history", true);
  }, [
    config.actions,
    config.state.dirty,
    config.state.page,
    config.state.saving,
    history.actions,
    history.refs.pageIndexRef,
    notice,
  ]);

  useEffect(() => {
    const action = config.refs.completedPendingRef.current;
    config.refs.completedPendingRef.current = null;
    if (action?.kind === "page" && action.page === "history" && action.resetHistoryPage) {
      history.refs.pageIndexRef.current = 0;
      void history.actions.loadPage(0).catch((error) => notice.show(String(error)));
    }
  }, [
    config.refs.completedPendingRef,
    config.state.page,
    history.actions,
    history.refs.pageIndexRef,
    notice,
  ]);

  useEffect(() => {
    config.actions.loadInitial()
      .then(async () => {
        const results = await Promise.allSettled([
          history.actions.refreshRecent(),
          history.actions.refreshStats(),
          system.actions.loadDataDir(),
          permissions.actions.initializeAccessibility(),
        ]);
        const errors = results
          .filter((result): result is PromiseRejectedResult => result.status === "rejected")
          .map((result) => String(result.reason));
        if (errors.length > 0) {
          throw new Error(errors.join("; "));
        }
      })
      .catch((error) => notice.show(String(error)));
  }, []);

  const canSave = Boolean(config.state.config) && !busy;
  const canMutateHistory = !busy
    && workflow.status.mode !== "starting"
    && workflow.status.mode !== "recording"
    && workflow.status.mode !== "processing";

  return {
    audio,
    config,
    history,
    notice,
    permissions,
    system,
    workflow,
    view: {
      busy,
      canChangeDataDir: canSave && canMutateHistory,
      canMutateHistory,
      canSave,
      canSaveChanges: canSave && config.state.dirty,
      showSaveBar: Boolean(config.state.config) && (
        config.state.dirty
        || config.state.page === "models"
        || config.state.page === "correction"
        || config.state.page === "settings"
      ),
    },
    actions: { openHistoryPage },
  };
}
