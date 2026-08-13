import { useCallback, useEffect, useRef, useState } from "react";

const noticeDurationMs = 5_000;

interface QueuedNotice {
  id: number;
  message: string;
}

export function useNoticeController() {
  const [queue, setQueue] = useState<QueuedNotice[]>([]);
  const nextIdRef = useRef(0);
  const current = queue[0] ?? null;

  const show = useCallback((message: string) => {
    const normalized = message.trim();
    if (!normalized) {
      return;
    }
    nextIdRef.current += 1;
    setQueue((items) => [...items, { id: nextIdRef.current, message: normalized }]);
  }, []);

  const dismiss = useCallback((noticeId: number) => {
    setQueue((items) => items[0]?.id === noticeId ? items.slice(1) : items);
  }, []);

  useEffect(() => {
    if (!current) {
      return;
    }
    const timer = window.setTimeout(() => dismiss(current.id), noticeDurationMs);
    return () => window.clearTimeout(timer);
  }, [current?.id, dismiss]);

  return { current, dismiss, show };
}
