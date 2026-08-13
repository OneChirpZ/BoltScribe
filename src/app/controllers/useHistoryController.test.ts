import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { HistoryRecord } from "../../types";
import { translations } from "../../domain/i18n";
import type { Page } from "../../domain/navigation";
import { createBrowserPreviewAppApi } from "../api/browserPreviewAppApi";
import { useHistoryController } from "./useHistoryController";

describe("useHistoryController", () => {
  it("keeps the newest pagination response when requests finish out of order", async () => {
    const api = createBrowserPreviewAppApi({ appVersion: "test-version" });
    const [seed] = await api.history.loadHistory(1);
    const older = { ...seed, id: "older" };
    const newest = { ...seed, id: "newest" };
    const olderRequest = deferred<HistoryRecord[]>();
    const newestRequest = deferred<HistoryRecord[]>();
    vi.spyOn(api.history, "loadHistory").mockImplementation((_limit, offset = 0) => (
      offset === 20 ? olderRequest.promise : newestRequest.promise
    ));
    const pageRef = { current: "home" as Page };
    const { result } = renderHook(() => useHistoryController({
      api,
      page: "home",
      pageRef,
      setBusy: vi.fn(),
      setNotice: vi.fn(),
      text: translations["en-US"],
    }));

    let olderPromise!: Promise<HistoryRecord[] | undefined>;
    let newestPromise!: Promise<HistoryRecord[] | undefined>;
    act(() => {
      olderPromise = result.current.actions.loadPage(1);
      newestPromise = result.current.actions.loadPage(0);
    });
    await act(async () => {
      newestRequest.resolve([newest]);
      await newestPromise;
    });
    await act(async () => {
      olderRequest.resolve([older]);
      await olderPromise;
    });

    expect(result.current.state.pageIndex).toBe(0);
    expect(result.current.state.records.map((record) => record.id)).toEqual(["newest"]);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}
