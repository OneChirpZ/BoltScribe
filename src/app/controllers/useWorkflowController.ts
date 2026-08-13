import { useCallback, useEffect, useState } from "react";
import type { WorkflowStatus } from "../../types";
import { emptyStatus, latestWorkflowStatus, subscribeToWorkflowStatus } from "../../domain/workflow";
import type { AppApi } from "../api";
import type { SetBusy, SetNotice } from "./controllerTypes";

export function useWorkflowController({
  api,
  setBusy,
  setNotice,
}: {
  api: AppApi;
  setBusy: SetBusy;
  setNotice: SetNotice;
}) {
  const [status, setStatus] = useState<WorkflowStatus>(emptyStatus);

  useEffect(() => subscribeToWorkflowStatus({
    listen: api.workflow.listenWorkflowStatus,
    getSnapshot: api.workflow.getStatus,
    onStatus: (nextStatus) => setStatus((current) => latestWorkflowStatus(current, nextStatus)),
    onError: (error) => setNotice(String(error)),
  }), [api, setNotice]);

  const toggleRecording = useCallback(async () => {
    setBusy(true);
    try {
      const nextStatus = await api.workflow.toggleRecording();
      setStatus((current) => latestWorkflowStatus(current, nextStatus));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, setBusy, setNotice]);

  const cancel = useCallback(async () => {
    setBusy(true);
    try {
      const nextStatus = await api.workflow.cancelCurrentWorkflow();
      setStatus((current) => latestWorkflowStatus(current, nextStatus));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }, [api, setBusy, setNotice]);

  return { status, actions: { cancel, toggleRecording } };
}
