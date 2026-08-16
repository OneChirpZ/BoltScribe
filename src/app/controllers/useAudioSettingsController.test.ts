import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { translations } from "../../domain/i18n";
import { createBrowserPreviewAppApi } from "../api/browserPreviewAppApi";
import { createBrowserPreviewState } from "../api/previewData";
import { useAudioSettingsController } from "./useAudioSettingsController";

describe("useAudioSettingsController", () => {
  it("waits for an in-flight VAD start before sending stop", async () => {
    const state = createBrowserPreviewState();
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    let resolveStart!: (status: typeof state.vadTestStatus) => void;
    const startVadTest = vi.spyOn(api.audio, "startVadTest").mockImplementation(() => new Promise((resolve) => {
      resolveStart = resolve;
    }));
    const stopVadTest = vi.spyOn(api.audio, "stopVadTest").mockResolvedValue({
      ...state.vadTestStatus,
      revision: 2,
    });
    const { result } = renderHook(() => useAudioSettingsController({
      api,
      busy: false,
      config: state.config,
      setBusy: vi.fn(),
      setNotice: vi.fn(),
      text: translations["zh-CN"],
      workflowStatus: state.status,
    }));

    act(() => {
      void result.current.actions.startVadTest();
    });
    expect(startVadTest).toHaveBeenCalledOnce();

    act(() => {
      void result.current.actions.stopVadTest();
    });
    expect(stopVadTest).not.toHaveBeenCalled();

    act(() => {
      resolveStart({
        ...state.vadTestStatus,
        mode: "listening",
        revision: 1,
      });
    });

    await waitFor(() => expect(stopVadTest).toHaveBeenCalledOnce());
    await waitFor(() => expect(result.current.state.vadStatus.revision).toBe(2));
    expect(result.current.state.vadStatus.mode).toBe("idle");
  });
});
