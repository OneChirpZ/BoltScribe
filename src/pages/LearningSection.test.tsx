import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LearningApi } from "../app/api";
import { createBrowserPreviewState } from "../app/api/previewData";
import { emptyLearningStatus } from "../domain/learning";
import type { AppConfig, LearnedEntry } from "../types";
import LearningSection from "./LearningSection";

function harness() {
  const preview = createBrowserPreviewState();
  let entries: LearnedEntry[] = preview.learnedEntries;
  let updated = () => {};
  const stop = vi.fn();
  const api: LearningApi = {
    loadActivity: vi.fn(async () => ({ active: [], history: [], has_more: false })),
    loadSource: vi.fn(async () => null),
    loadEntries: vi.fn(async () => entries),
    getStatus: vi.fn(async () => emptyLearningStatus()),
    deleteEntry: vi.fn(async (id) => {
      entries = entries.filter((entry) => entry.id !== id);
      return entries;
    }),
    setEntryEnabled: vi.fn(async (id, enabled) => {
      entries = entries.map((entry) => entry.id === id ? { ...entry, enabled } : entry);
      return entries;
    }),
    listenUpdated: vi.fn(async (handler) => { updated = handler; return stop; }),
    listenOpenRequested: vi.fn(async () => () => undefined),
    takeOpenRequest: vi.fn(async () => false),
  };
  return {
    config: preview.config, api, onChange: vi.fn(), onOpenModels: vi.fn(), stop,
    notify: () => updated(),
    replaceEntries: (next: LearnedEntry[]) => { entries = next; },
  };
}

afterEach(() => { vi.useRealTimers(); });

describe("LearningSection", () => {
  it("defaults learning off and links to centralized model settings", async () => {
    const props = harness();
    render(<LearningSection {...props} />);
    await screen.findByText("Kubernetes");
    expect((screen.getByRole("checkbox", { name: "自动学习纠错词" }) as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByText("配置学习模型后自动生效。")).toBeNull();
    expect(screen.queryByLabelText("API Key")).toBeNull();
    expect(screen.queryByLabelText("模型名称")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "管理模型" }));
    expect(props.onOpenModels).toHaveBeenCalledOnce();
    expect(props.onChange).not.toHaveBeenCalled();
  });

  it("shows the selected shared service without duplicating its credentials", async () => {
    const props = harness();
    props.config.learning.llm.model = "quality-model";
    props.config.model_services[0].name = "共享服务";
    props.config.model_services[0].api_key = "shared-test-key";
    render(<LearningSection {...props} />);
    await screen.findByText("Kubernetes");
    expect(screen.getByText("共享服务 / quality-model")).toBeTruthy();
    expect(screen.queryByText("配置学习模型后自动生效。")).toBeNull();
    expect(screen.queryByDisplayValue("shared-test-key")).toBeNull();
    expect(screen.queryByLabelText("API Key")).toBeNull();
  });

  it("disables automatic observation on unsupported platforms while retaining review actions", async () => {
    const props = harness();
    props.config.learning.enabled = false;
    const status = emptyLearningStatus();
    status.observation.supported = false;
    vi.mocked(props.api.getStatus).mockResolvedValue(status);
    render(<LearningSection {...props} />);
    await screen.findByText("当前平台暂不支持自动观察输入；已学习词条仍可管理。");
    expect((screen.getByRole("checkbox", { name: "自动学习纠错词" }) as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "删除 Kubernetes" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("allows an imported enabled setting to be switched off on an unsupported platform", async () => {
    const props = harness();
    props.config.learning.enabled = true;
    const status = emptyLearningStatus();
    status.observation.supported = false;
    vi.mocked(props.api.getStatus).mockResolvedValue(status);
    const view = render(<LearningSection {...props} />);
    await screen.findByText("当前平台暂不支持自动观察输入；已学习词条仍可管理。");
    const toggle = screen.getByRole("checkbox", { name: "自动学习纠错词" }) as HTMLInputElement;
    expect(toggle.disabled).toBe(false);
    fireEvent.click(toggle);
    const next: AppConfig = props.onChange.mock.calls[0][0];
    expect(next.learning.enabled).toBe(false);
    view.rerender(<LearningSection {...props} config={next} />);
    expect(toggle.disabled).toBe(true);
  });

  it("reviews, disables and deletes learned entries without editing the manual dictionary", async () => {
    const props = harness();
    render(<LearningSection {...props} />);
    await screen.findByText("库伯内特斯");
    fireEvent.click(screen.getByRole("checkbox", { name: "启用 Kubernetes" }));
    await waitFor(() => expect((screen.getByRole("checkbox", { name: "启用 Kubernetes" }) as HTMLInputElement).checked).toBe(false));
    expect(props.api.setEntryEnabled).toHaveBeenCalledWith("preview-learning-1", false);
    await waitFor(() => expect((screen.getByRole("button", { name: "删除 Kubernetes" }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "删除 Kubernetes" }));
    await waitFor(() => expect(screen.queryByText("Kubernetes")).toBeNull());
    expect(props.api.deleteEntry).toHaveBeenCalledWith("preview-learning-1");
    expect(props.onChange).not.toHaveBeenCalled();
    expect(screen.getByText("DemoTerm")).toBeTruthy();
  });

  it("refreshes on learning events and releases the subscription on unmount", async () => {
    const props = harness();
    const view = render(<LearningSection {...props} />);
    await screen.findByText("Kubernetes");
    props.replaceEntries([]);
    act(() => props.notify());
    await screen.findByText("还没有自学习词条。");
    expect(props.api.loadEntries).toHaveBeenCalledTimes(2);
    view.unmount();
    expect(props.stop).toHaveBeenCalledTimes(1);
  });

  it("does not poll while the page is idle", async () => {
    vi.useFakeTimers();
    const props = harness();
    render(<LearningSection {...props} />);
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(props.api.loadEntries).toHaveBeenCalledTimes(1);
    expect(props.api.getStatus).toHaveBeenCalledTimes(1);
  });

  it("releases a subscription that resolves after unmount without reading data", async () => {
    const props = harness();
    let resolveSubscription!: (stop: () => void) => void;
    vi.mocked(props.api.listenUpdated).mockImplementation(() => new Promise((resolve) => { resolveSubscription = resolve; }));
    const view = render(<LearningSection {...props} />);
    view.unmount();
    await act(async () => { resolveSubscription(props.stop); });
    expect(props.stop).toHaveBeenCalledTimes(1);
    expect(props.api.loadEntries).not.toHaveBeenCalled();
  });

  it("keeps a failed deletion visible and shows its error", async () => {
    const props = harness();
    vi.mocked(props.api.deleteEntry).mockRejectedValue(new Error("storage unavailable"));
    render(<LearningSection {...props} />);
    await screen.findByText("Kubernetes");
    fireEvent.click(screen.getByRole("button", { name: "删除 Kubernetes" }));
    expect((await screen.findByRole("alert")).textContent).toContain("storage unavailable");
    expect(screen.getByText("Kubernetes")).toBeTruthy();
  });
});
