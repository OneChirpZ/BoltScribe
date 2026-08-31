import { describe, expect, it } from "vitest";
import type { AppApi } from "./contracts";
import { createBrowserPreviewAppApi } from "./browserPreviewAppApi";
import { createBrowserPreviewState } from "./previewData";
import { nativeAppApi } from "./nativeAppApi";

describe("browser preview AppApi", () => {
  it("keeps native and preview adapter surfaces in sync", () => {
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    const domains = Object.keys(nativeAppApi) as Array<keyof AppApi>;

    expect(domains).toEqual([
      "config",
      "overlay",
      "workflow",
      "history",
      "audio",
      "permissions",
      "system",
    ]);
    for (const domain of domains) {
      expect(Object.keys(api[domain]).sort()).toEqual(Object.keys(nativeAppApi[domain]).sort());
    }
  });

  it("keeps preview config snapshots isolated from callers", async () => {
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    const first = await api.config.loadConfig();
    first.hotkeys.push("F8");

    const second = await api.config.loadConfig();
    expect(second.hotkeys).not.toContain("F8");

    second.ui.app_language = "en-US";
    const saved = await api.config.saveConfig(second);
    saved.config.ui.app_language = "zh-CN";
    expect(saved.warnings).toEqual([]);
    expect((await api.config.loadConfig()).ui.app_language).toBe("en-US");
  });

  it("shares workflow state across workflow and audio domains", async () => {
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    const recording = await api.workflow.toggleRecording();
    const samples: number[] = [];
    const unlisten = await api.audio.listenAudioLevel((sample) => samples.push(sample.recording_revision));

    expect(recording.mode).toBe("recording");
    expect(samples[0]).toBe(recording.revision);
    unlisten();
    expect((await api.workflow.cancelCurrentWorkflow()).mode).toBe("idle");
  });

  it("supports a recognizing overlay preview", () => {
    const state = createBrowserPreviewState("http://localhost/?preview-status=recognizing");

    expect(state.status).toMatchObject({
      mode: "processing",
      stage: "recognizing",
      code: "recognizing",
    });
  });

  it("supports bounded overlay scales and terminal visual states for QA", () => {
    const complete = createBrowserPreviewState("http://localhost/?preview-status=complete&preview-overlay-scale=1");
    const compact = createBrowserPreviewState("http://localhost/?preview-overlay-scale=0.1");

    expect(complete.status).toMatchObject({ mode: "processing", stage: "complete" });
    expect(complete.config.ui.recording_overlay_scale).toBe(1);
    expect(compact.config.ui.recording_overlay_scale).toBe(0.25);
  });
});
