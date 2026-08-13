import { useEffect, useRef, useState } from "react";
import { cleanupCutoffTimestamp } from "../../domain/historyMaintenance";
import type { RecordingCleanupPreview, RecordingCleanupUnit } from "../../types";

export function useRecordingCleanup({
  dataDirPath,
  onCleanup,
  onPreview,
}: {
  dataDirPath: string | undefined;
  onCleanup: (amount: number, unit: RecordingCleanupUnit) => Promise<void>;
  onPreview: (amount: number, unit: RecordingCleanupUnit) => Promise<RecordingCleanupPreview>;
}) {
  const [amount, setAmount] = useState("1");
  const [unit, setUnit] = useState<RecordingCleanupUnit>("week");
  const [cleaning, setCleaning] = useState(false);
  const [preview, setPreview] = useState<RecordingCleanupPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState(false);
  const [previewRevision, setPreviewRevision] = useState(0);
  const requestRef = useRef(0);
  const amountNumber = Number(amount);
  const amountValid = cleanupCutoffTimestamp(Date.now(), amountNumber, unit) !== null;

  useEffect(() => {
    requestRef.current += 1;
    const requestId = requestRef.current;
    if (!amountValid) {
      setPreviewLoading(false);
      setPreviewError(false);
      return;
    }
    const timer = window.setTimeout(() => {
      setPreviewLoading(true);
      setPreviewError(false);
      onPreview(amountNumber, unit)
        .then((nextPreview) => {
          if (requestRef.current === requestId) {
            setPreview(nextPreview);
          }
        })
        .catch(() => {
          if (requestRef.current === requestId) {
            setPreviewError(true);
          }
        })
        .finally(() => {
          if (requestRef.current === requestId) {
            setPreviewLoading(false);
          }
        });
    }, 150);
    return () => window.clearTimeout(timer);
  }, [amountNumber, unit, amountValid, previewRevision, dataDirPath, onPreview]);

  async function runCleanup() {
    if (!amountValid) {
      return;
    }
    setCleaning(true);
    try {
      await onCleanup(amountNumber, unit);
      setPreviewRevision((revision) => revision + 1);
    } finally {
      setCleaning(false);
    }
  }

  return {
    amount,
    amountNumber,
    amountValid,
    cleaning,
    runCleanup,
    preview,
    previewError,
    previewLoading,
    setAmount,
    setUnit,
    unit,
  };
}
