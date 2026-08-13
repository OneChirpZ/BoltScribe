import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import type { HistoryRecord, InputStats, RecordingCleanupUnit } from "../../types";
import { formatByteCount } from "../../domain/historyMaintenance";
import type { Page } from "../../domain/navigation";
import type { AppApi } from "../api";
import type { AppText, SetBusy, SetNotice } from "./controllerTypes";

const recentHistoryLimit = 6;
export const historyPageSize = 20;

export function useHistoryController({
  api,
  page,
  pageRef,
  setBusy,
  setNotice,
  text,
}: {
  api: AppApi;
  page: Page;
  pageRef: MutableRefObject<Page>;
  setBusy: SetBusy;
  setNotice: SetNotice;
  text: AppText;
}) {
  const [recent, setRecent] = useState<HistoryRecord[]>([]);
  const [stats, setStats] = useState<InputStats | null>(null);
  const [records, setRecords] = useState<HistoryRecord[]>([]);
  const [pageIndex, setPageIndex] = useState(0);
  const [hasOlder, setHasOlder] = useState(false);
  const pageIndexRef = useRef(pageIndex);
  const pageRequestRef = useRef(0);

  const refreshRecent = useCallback(async () => {
    const nextRecords = await api.history.loadHistory(recentHistoryLimit);
    setRecent(nextRecords);
    return nextRecords;
  }, [api]);

  const refreshStats = useCallback(async () => {
    const nextStats = await api.history.loadStats();
    setStats(nextStats);
    return nextStats;
  }, [api]);

  const loadPage = useCallback(async (nextPageIndex: number) => {
    const request = pageRequestRef.current + 1;
    pageRequestRef.current = request;
    pageIndexRef.current = nextPageIndex;
    let loaded: HistoryRecord[];
    try {
      loaded = await api.history.loadHistory(historyPageSize + 1, nextPageIndex * historyPageSize);
    } catch (error) {
      if (request !== pageRequestRef.current) {
        return undefined;
      }
      throw error;
    }
    if (request !== pageRequestRef.current) {
      return undefined;
    }
    const pageRecords = loaded.slice(0, historyPageSize);
    setPageIndex(nextPageIndex);
    setRecords(pageRecords);
    setHasOlder(loaded.length > historyPageSize);
    return pageRecords;
  }, [api]);

  const refreshViews = useCallback(async (fallbackFromEmptyPage = false) => {
    await refreshRecent();
    if (pageRef.current !== "history") {
      return;
    }
    const currentPage = pageIndexRef.current;
    const pageRecords = await loadPage(currentPage);
    if (fallbackFromEmptyPage && pageRecords?.length === 0 && currentPage > 0) {
      await loadPage(currentPage - 1);
    }
  }, [loadPage, pageRef, refreshRecent]);

  const copyText = useCallback(async (value: string, label: string) => {
    if (!value.trim()) {
      setNotice(text.notices.emptyCopy(label));
      return;
    }
    try {
      await api.system.copyTextToClipboard(value);
      setNotice(text.notices.copied(label));
    } catch (error) {
      setNotice(String(error));
    }
  }, [api, setNotice, text.notices]);

  const deleteRecord = useCallback(async (record: HistoryRecord) => {
    setBusy(true);
    try {
      const result = await api.history.deleteHistoryRecord(record.id);
      await refreshViews(true);
      setNotice(result.deleted_records === 0
        ? text.notices.historyDeleteNotFound
        : text.notices.historyDeleted(
          result.deleted_records,
          result.deleted_audio_files,
          formatByteCount(result.freed_bytes),
        ));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, refreshViews, setBusy, setNotice, text.notices]);

  const retryRecord = useCallback(async (record: HistoryRecord) => {
    setBusy(true);
    try {
      const updatedRecord = await api.history.retryHistoryRecord(record.id);
      setRecent((items) => replaceHistoryRecord(items, updatedRecord));
      setRecords((items) => replaceHistoryRecord(items, updatedRecord));
      await refreshStats().catch(() => undefined);
      setNotice(text.notices.historyRetrySucceeded);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setNotice(text.notices.historyRetryFailed(message));
    } finally {
      setBusy(false);
    }
  }, [api, refreshStats, setBusy, setNotice, text.notices]);

  const cleanupRecordings = useCallback(async (amount: number, unit: RecordingCleanupUnit) => {
    setBusy(true);
    try {
      const result = await api.history.cleanupRecordingFiles(amount, unit);
      await refreshViews();
      setNotice(result.deleted_files === 0 && result.cleared_history_records === 0
        ? text.notices.noRecordingsCleaned
        : text.notices.recordingsCleaned(
          result.deleted_files,
          result.cleared_history_records,
          formatByteCount(result.freed_bytes),
        ));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, refreshViews, setBusy, setNotice, text.notices]);

  const refreshAfterDirectoryChange = useCallback(async () => {
    await Promise.all([
      refreshRecent(),
      refreshStats(),
      pageRef.current === "history" ? loadPage(pageIndexRef.current) : Promise.resolve(),
    ]);
  }, [loadPage, pageRef, refreshRecent, refreshStats]);

  useEffect(() => {
    pageIndexRef.current = pageIndex;
  }, [pageIndex]);

  useEffect(() => {
    if (page === "history") {
      loadPage(pageIndexRef.current).catch((error) => setNotice(String(error)));
    }
  }, [page]);

  useEffect(() => {
    const unlisten = api.history.listenHistoryUpdated(() => {
      refreshViews(true).catch((error) => setNotice(String(error)));
      refreshStats().catch((error) => setNotice(String(error)));
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [api, refreshStats, refreshViews, setNotice]);

  return {
    state: { recent, stats, records, pageIndex, hasOlder },
    refs: { pageIndexRef },
    actions: {
      changePage: loadPage,
      cleanupRecordings,
      copyText,
      deleteRecord,
      loadPage,
      previewCleanup: api.history.previewRecordingCleanup,
      refreshAfterDirectoryChange,
      refreshRecent,
      refreshStats,
      refreshViews,
      retryRecord,
    },
  };
}

function replaceHistoryRecord(records: HistoryRecord[], updatedRecord: HistoryRecord) {
  return records.map((record) => record.id === updatedRecord.id ? updatedRecord : record);
}
