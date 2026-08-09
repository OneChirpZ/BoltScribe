import assert from "node:assert/strict";
import test from "node:test";
import type { WorkflowStatus } from "../src/types.ts";
import { latestWorkflowStatus, subscribeToWorkflowStatus } from "../src/domain/workflow.ts";

function status(mode: WorkflowStatus["mode"], revision: number): WorkflowStatus {
  const stage = mode === "starting" ? "starting" : mode === "recording" ? "recording" : mode === "error" ? "error" : "idle";
  return {
    mode,
    stage,
    message: mode,
    current_audio_path: null,
    last_record_id: null,
    revision,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
}

test("workflow status subscription registers the listener before reading the snapshot", async () => {
  const order: string[] = [];
  const received: WorkflowStatus[] = [];

  const stop = subscribeToWorkflowStatus({
    listen: async () => {
      order.push("listen");
      return () => undefined;
    },
    getSnapshot: async () => {
      order.push("snapshot");
      return status("idle", 0);
    },
    onStatus: (next) => received.push(next),
  });

  await flushPromises();
  assert.deepEqual(order, ["listen", "snapshot"]);
  assert.equal(received.at(-1)?.mode, "idle");
  stop();
});

test("an older snapshot cannot overwrite a newer workflow event", async () => {
  const snapshot = deferred<WorkflowStatus>();
  const received: WorkflowStatus[] = [];
  let handler: ((next: WorkflowStatus) => void) | undefined;

  const stop = subscribeToWorkflowStatus({
    listen: async (next) => {
      handler = next;
      return () => undefined;
    },
    getSnapshot: () => snapshot.promise,
    onStatus: (next) => received.push(next),
  });

  await flushPromises();
  handler?.(status("recording", 2));
  snapshot.resolve(status("idle", 1));
  await flushPromises();

  assert.deepEqual(received.map((next) => next.mode), ["recording"]);
  stop();
});

test("disposing a pending workflow subscription unregisters it without reading a snapshot", async () => {
  const listener = deferred<() => void>();
  let unlistenCount = 0;
  let snapshotCount = 0;

  const stop = subscribeToWorkflowStatus({
    listen: () => listener.promise,
    getSnapshot: async () => {
      snapshotCount += 1;
      return status("idle", 0);
    },
    onStatus: () => undefined,
  });

  stop();
  listener.resolve(() => {
    unlistenCount += 1;
  });
  await flushPromises();

  assert.equal(unlistenCount, 1);
  assert.equal(snapshotCount, 0);
});

test("a listener registration failure still falls back to the current snapshot", async () => {
  const received: WorkflowStatus[] = [];
  const errors: unknown[] = [];

  const stop = subscribeToWorkflowStatus({
    listen: async () => {
      throw new Error("listener unavailable");
    },
    getSnapshot: async () => status("recording", 3),
    onStatus: (next) => received.push(next),
    onError: (error) => errors.push(error),
  });

  await flushPromises();

  assert.equal(errors.length, 1);
  assert.equal(received.at(-1)?.mode, "recording");
  stop();
});

test("latest workflow status keeps the highest revision", () => {
  const recording = status("recording", 4);

  assert.equal(latestWorkflowStatus(recording, status("idle", 3)), recording);
  assert.equal(latestWorkflowStatus(recording, status("processing", 5)).mode, "processing");
});
