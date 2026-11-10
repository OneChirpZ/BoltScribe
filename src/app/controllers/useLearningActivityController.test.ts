import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { LearningApi } from "../api";
import type { LearningActivityPage } from "../../types";
import { useLearningActivityController } from "./useLearningActivityController";

function harness() {
  let updated = () => {};
  let open = () => {};
  const stopUpdated = vi.fn();
  const stopOpen = vi.fn();
  const api = {
    loadActivity: vi.fn().mockResolvedValue({ active: [], history: [], has_more: false }),
    takeOpenRequest: vi.fn().mockResolvedValue(false),
    listenUpdated: vi.fn(async (handler: () => void) => {
      updated = handler;
      return stopUpdated;
    }),
    listenOpenRequested: vi.fn(async (handler: () => void) => {
      open = handler;
      return stopOpen;
    }),
  } as unknown as LearningApi;
  return { api, stopUpdated, stopOpen, updated: () => updated(), open: () => open() };
}

describe("learning sidebar activity", () => {
  it("opens a pending tray request on startup and uses the latest navigation callback", async () => {
    const h = harness();
    vi.mocked(h.api.takeOpenRequest).mockResolvedValueOnce(true);
    const firstOpen = vi.fn();
    const nextOpen = vi.fn();
    const { rerender, unmount } = renderHook(({ onOpen }) => useLearningActivityController(h.api, onOpen), {
      initialProps: { onOpen: firstOpen },
    });
    await waitFor(() => expect(firstOpen).toHaveBeenCalledOnce());
    rerender({ onOpen: nextOpen });
    vi.mocked(h.api.takeOpenRequest).mockResolvedValueOnce(true);
    act(() => h.open());
    await waitFor(() => expect(nextOpen).toHaveBeenCalledOnce());
    expect(h.api.listenOpenRequested).toHaveBeenCalledOnce();
    unmount();
    expect(h.stopUpdated).toHaveBeenCalledOnce();
    expect(h.stopOpen).toHaveBeenCalledOnce();
  });

  it("keeps the newer event snapshot when startup refresh resolves later", async () => {
    const h = harness();
    let resolveInitial: (value: LearningActivityPage) => void = () => {};
    vi.mocked(h.api.loadActivity).mockImplementationOnce(() => new Promise((resolve) => {
      resolveInitial = resolve;
    }));
    const { result } = renderHook(() => useLearningActivityController(h.api, vi.fn()));
    await waitFor(() => expect(h.api.loadActivity).toHaveBeenCalledOnce());
    vi.mocked(h.api.loadActivity).mockRejectedValueOnce(new Error("unavailable"));
    act(() => h.updated());
    await waitFor(() => expect(result.current.error).toContain("unavailable"));
    await act(async () => resolveInitial({ active: [], history: [], has_more: false }));
    expect(result.current.error).toContain("unavailable");
    vi.mocked(h.api.loadActivity).mockResolvedValueOnce({ active: [], history: [], has_more: false });
    act(() => h.updated());
    await waitFor(() => expect(result.current.error).toBeNull());
  });
});
