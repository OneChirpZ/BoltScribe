import type { WorkflowStatus } from "../types";

export const emptyStatus: WorkflowStatus = {
  mode: "idle",
  message: "就绪",
  current_audio_path: null,
  last_record_id: null,
  revision: 0,
};

export function latestWorkflowStatus(current: WorkflowStatus, next: WorkflowStatus) {
  return next.revision >= current.revision ? next : current;
}

export function subscribeToWorkflowStatus({
  listen,
  getSnapshot,
  onStatus,
  onError,
}: {
  listen: (handler: (status: WorkflowStatus) => void) => Promise<() => void>;
  getSnapshot: () => Promise<WorkflowStatus>;
  onStatus: (status: WorkflowStatus) => void;
  onError?: (error: unknown) => void;
}) {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  let latestRevision = -1;

  const accept = (status: WorkflowStatus) => {
    if (disposed || status.revision < latestRevision) {
      return;
    }
    latestRevision = status.revision;
    onStatus(status);
  };

  void (async () => {
    try {
      const stopListening = await listen(accept);
      if (disposed) {
        stopListening();
        return;
      }
      unlisten = stopListening;
    } catch (error) {
      if (!disposed) {
        onError?.(error);
      }
    }

    if (disposed) {
      return;
    }
    try {
      accept(await getSnapshot());
    } catch (error) {
      if (!disposed) {
        onError?.(error);
      }
    }
  })();

  return () => {
    disposed = true;
    unlisten?.();
  };
}
