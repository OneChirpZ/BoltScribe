import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RecordingOverlayPositionUpdate, SaveConfigResult } from "../../types";
import { createBrowserPreviewAppApi } from "../api/browserPreviewAppApi";
import { useConfigController } from "./useConfigController";

describe("useConfigController", () => {
  beforeEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn((query: string): MediaQueryList => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(() => false),
      })),
    });
    Object.defineProperty(window, "requestAnimationFrame", {
      configurable: true,
      value: (_callback: FrameRequestCallback) => 1,
    });
    Object.defineProperty(window, "cancelAnimationFrame", {
      configurable: true,
      value: (_handle: number) => undefined,
    });
  });

  it("treats warnings as a successful save before completing pending navigation", async () => {
    const api = createBrowserPreviewAppApi({
      appVersion: "test-version",
      locationHref: "http://localhost/?preview-language=en-US",
    });
    const saveConfig = vi.spyOn(api.config, "saveConfig").mockImplementation(async (config) => ({
      config,
      warnings: [{ code: "tray_sync_failed", detail: "Tray refresh failed" }],
    }));
    const setBusy = vi.fn();
    const setNotice = vi.fn();
    const { result } = renderHook(() => useConfigController({ api, busy: false, setBusy, setNotice }));

    await act(async () => {
      await result.current.actions.loadInitial();
    });
    const config = result.current.state.config;
    expect(config).not.toBeNull();

    act(() => {
      result.current.actions.change({
        ...config!,
        ui: { ...config!.ui, theme: "dark" },
      });
    });
    act(() => {
      result.current.actions.requestPageChange("history");
    });

    expect(result.current.state.dirty).toBe(true);
    expect(result.current.state.pending).toEqual({ kind: "page", page: "history" });
    expect(result.current.state.page).toBe("home");

    await act(async () => {
      await result.current.actions.savePending();
    });

    expect(saveConfig).toHaveBeenCalledOnce();
    expect(result.current.state.dirty).toBe(false);
    expect(result.current.state.pending).toBeNull();
    expect(result.current.state.page).toBe("history");
    expect(setNotice).toHaveBeenLastCalledWith("Tray refresh failed");
    expect(setBusy.mock.calls).toEqual([[true], [false]]);
  });

  it("ignores navigation requests while a manual save is in flight", async () => {
    const api = createBrowserPreviewAppApi({
      appVersion: "test-version",
      locationHref: "http://localhost/?preview-language=en-US",
    });
    let resolveSave!: (result: SaveConfigResult) => void;
    vi.spyOn(api.config, "saveConfig").mockImplementation(() => new Promise((resolve) => {
      resolveSave = resolve;
    }));
    const { result } = renderHook(() => useConfigController({
      api,
      busy: false,
      setBusy: vi.fn(),
      setNotice: vi.fn(),
    }));

    await act(async () => {
      await result.current.actions.loadInitial();
    });
    const config = result.current.state.config!;
    const next = { ...config, ui: { ...config.ui, theme: "dark" as const } };
    act(() => {
      result.current.actions.change(next);
    });

    let savePromise!: Promise<void>;
    act(() => {
      savePromise = result.current.actions.save(next);
    });
    act(() => {
      result.current.actions.requestPageChange("history");
      result.current.actions.requestWindowClose();
    });

    expect(result.current.state.saving).toBe(true);
    expect(result.current.state.page).toBe("home");
    expect(result.current.state.pending).toBeNull();

    await act(async () => {
      resolveSave({ config: next, warnings: [] });
      await savePromise;
    });
  });

  it("keeps the draft and pending navigation when a fatal save fails", async () => {
    const api = createBrowserPreviewAppApi({
      appVersion: "test-version",
      locationHref: "http://localhost/?preview-language=en-US",
    });
    const saveConfig = vi.spyOn(api.config, "saveConfig")
      .mockRejectedValue(new Error("Shortcut registration failed"));
    const setNotice = vi.fn();
    const { result } = renderHook(() => useConfigController({
      api,
      busy: false,
      setBusy: vi.fn(),
      setNotice,
    }));

    await act(async () => {
      await result.current.actions.loadInitial();
    });
    const config = result.current.state.config;
    expect(config).not.toBeNull();

    act(() => {
      result.current.actions.change({
        ...config!,
        system: { ...config!.system, launch_at_login: !config!.system.launch_at_login },
      });
    });
    act(() => {
      result.current.actions.requestPageChange("settings");
    });

    await act(async () => {
      await result.current.actions.savePending();
    });

    expect(saveConfig).toHaveBeenCalledOnce();
    expect(result.current.state.dirty).toBe(true);
    expect(result.current.state.pending).toEqual({ kind: "page", page: "settings" });
    expect(result.current.state.page).toBe("home");
    expect(result.current.state.saving).toBe(false);
    expect(setNotice).toHaveBeenLastCalledWith("Error: Shortcut registration failed");
  });

  it("does not overwrite edits made while an older save is pending", async () => {
    const api = createBrowserPreviewAppApi({
      appVersion: "test-version",
      locationHref: "http://localhost/?preview-language=en-US",
    });
    let resolveSave!: (result: SaveConfigResult) => void;
    vi.spyOn(api.config, "saveConfig").mockImplementation((config) => new Promise((resolve) => {
      resolveSave = resolve;
    }));
    const { result } = renderHook(() => useConfigController({
      api,
      busy: false,
      setBusy: vi.fn(),
      setNotice: vi.fn(),
    }));

    await act(async () => {
      await result.current.actions.loadInitial();
    });
    const original = result.current.state.config!;
    const saving = { ...original, ui: { ...original.ui, theme: "dark" as const } };
    const newer = { ...saving, ui: { ...saving.ui, app_language: "zh-CN" as const } };

    let savePromise!: Promise<void>;
    act(() => {
      savePromise = result.current.actions.save(saving);
    });
    act(() => {
      result.current.actions.change(newer);
    });
    await act(async () => {
      resolveSave({ config: saving, warnings: [] });
      await savePromise;
    });

    expect(result.current.state.config).toEqual(newer);
    expect(result.current.state.savedConfig).toEqual(saving);
    expect(result.current.state.dirty).toBe(true);
  });

  it("merges direct overlay positions into both saved and dirty draft state", async () => {
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    let positionHandler: ((update: RecordingOverlayPositionUpdate) => void) | undefined;
    vi.spyOn(api.overlay, "listenRecordingOverlayPositionUpdated").mockImplementation((handler) => {
      positionHandler = handler;
      return Promise.resolve(() => undefined);
    });
    const { result } = renderHook(() => useConfigController({
      api,
      busy: false,
      setBusy: vi.fn(),
      setNotice: vi.fn(),
    }));

    await act(async () => {
      await result.current.actions.loadInitial();
    });
    const original = result.current.state.config!;
    act(() => {
      result.current.actions.change({
        ...original,
        system: { ...original.system, hide_dock_icon: !original.system.hide_dock_icon },
      });
    });
    act(() => {
      positionHandler?.({
        recording_overlay_offset_x: 240,
        recording_overlay_offset_y: -80,
      });
    });

    expect(result.current.state.config?.system.hide_dock_icon).toBe(!original.system.hide_dock_icon);
    expect(result.current.state.savedConfig?.system.hide_dock_icon).toBe(original.system.hide_dock_icon);
    expect(result.current.state.config?.ui.recording_overlay_offset_x).toBe(240);
    expect(result.current.state.savedConfig?.ui.recording_overlay_offset_x).toBe(240);
    expect(result.current.state.config?.ui.recording_overlay_offset_y).toBe(-80);
    expect(result.current.state.savedConfig?.ui.recording_overlay_offset_y).toBe(-80);
    expect(result.current.state.dirty).toBe(true);
  });
});
