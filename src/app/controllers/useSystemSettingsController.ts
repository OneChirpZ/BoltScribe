import { useCallback, useEffect, useState } from "react";
import type { DataDirInfo } from "../../types";
import type { AppApi } from "../api";
import type { AppText, SetBusy, SetNotice } from "./controllerTypes";

export function useSystemSettingsController({
  api,
  refreshHistoryAfterDirectoryChange,
  setBusy,
  setNotice,
  text,
}: {
  api: AppApi;
  refreshHistoryAfterDirectoryChange: () => Promise<void>;
  setBusy: SetBusy;
  setNotice: SetNotice;
  text: AppText;
}) {
  const [appVersion, setAppVersion] = useState("");
  const [dataDir, setDataDir] = useState<DataDirInfo | null>(null);

  const loadDataDir = useCallback(async () => {
    const info = await api.system.getDataDir();
    setDataDir(info);
    return info;
  }, [api]);

  const acceptDirectoryChange = useCallback(async (info: DataDirInfo) => {
    setDataDir(info);
    await refreshHistoryAfterDirectoryChange();
    setNotice(info.cleanup_warning ?? text.notices.dataDirChanged(info.path));
  }, [refreshHistoryAfterDirectoryChange, setNotice, text.notices]);

  const changeDataDir = useCallback(async () => {
    setBusy(true);
    try {
      const selectedPath = await api.system.chooseDataDir();
      if (selectedPath) {
        await acceptDirectoryChange(await api.system.setDataDir(selectedPath));
      }
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [acceptDirectoryChange, api, setBusy, setNotice]);

  const resetDataDir = useCallback(async () => {
    setBusy(true);
    try {
      await acceptDirectoryChange(await api.system.resetDataDir());
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [acceptDirectoryChange, api, setBusy, setNotice]);

  const openGitHub = useCallback(async () => {
    try {
      await api.system.openGitHubRepository();
    } catch (error) {
      setNotice(String(error));
    }
  }, [api, setNotice]);

  useEffect(() => {
    api.system.getAppVersion().then(setAppVersion).catch(() => setAppVersion(""));
  }, [api]);

  return {
    state: { appVersion, dataDir },
    actions: {
      changeDataDir,
      loadDataDir,
      openAppDir: api.system.openAppDir,
      openGitHub,
      resetDataDir,
    },
  };
}
