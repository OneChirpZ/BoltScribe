import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { translations } from "../../domain/i18n";
import type { VadTestStatus } from "../../types";
import { useVadTest } from "./useVadTest";

const idleStatus: VadTestStatus = {
  mode: "idle",
  raw_voice_active: false,
  voice_active: false,
  level: -96,
  noise_calibrated: false,
  noise_floor: -96,
  trigger_threshold: -96,
  trigger_progress: 0,
  elapsed_ms: 0,
  remaining_ms: 60_000,
  noise_margin_db: 12,
  confirmation_ms: 480,
  noise_window_ms: 800,
  revision: 0,
  error: null,
};

describe("useVadTest", () => {
  it("tracks active samples and stops an active test on unmount", () => {
    const stop = vi.fn();
    const { result, rerender, unmount } = renderHook(
      ({ status }) => useVadTest(status, stop, translations["en-US"]),
      { initialProps: { status: idleStatus } },
    );

    expect(result.current.running).toBe(false);
    act(() => {
      rerender({ status: { ...idleStatus, mode: "listening", level: -18, revision: 1 } });
    });
    expect(result.current.running).toBe(true);
    expect(result.current.levelHistory[result.current.levelHistory.length - 1]).toBeGreaterThan(0);

    unmount();
    expect(stop).toHaveBeenCalledOnce();
  });
});
