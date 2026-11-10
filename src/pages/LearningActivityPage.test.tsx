import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LearningApi } from "../app/api";
import { createBrowserPreviewState } from "../app/api/previewData";
import { emptyLearningStatus } from "../domain/learning";
import type { LearningActivityPage as ActivityPage } from "../types";
import LearningActivityPage from "./LearningActivityPage";

function harness() {
  const preview = createBrowserPreviewState("http://localhost/?preview-learning=active");
  let snapshot: ActivityPage = {
    active: preview.learningActivity.filter((item) => item.finished_at === null),
    history: preview.learningActivity.filter((item) => item.finished_at !== null),
    has_more: false,
  };
  let updated = () => {};
  const stop = vi.fn();
  const api: LearningApi = {
    loadActivity: vi.fn(async () => snapshot),
    loadSource: vi.fn(async () => null),
    loadEntries: vi.fn(async () => preview.learnedEntries),
    getStatus: vi.fn(async () => emptyLearningStatus()),
    deleteEntry: vi.fn(async () => []),
    setEntryEnabled: vi.fn(async () => []),
    listenUpdated: vi.fn(async (handler) => { updated = handler; return stop; }),
    listenOpenRequested: vi.fn(async () => () => {}),
    takeOpenRequest: vi.fn(async () => false),
  };
  return {
    config: preview.config, api, onChange: vi.fn(), onOpenModels: vi.fn(), onOpenRecord: vi.fn(), stop,
    notify: () => updated(),
    replace: (next: ActivityPage) => { snapshot = next; },
  };
}

afterEach(() => { vi.useRealTimers(); });

describe("LearningActivityPage", () => {
  it("opens the privacy explanation and returns focus when it closes", async () => {
    render(<LearningActivityPage {...harness()} />);
    await screen.findByText("正在观察上屏后的修改");
    const trigger = screen.getByRole("button", { name: "功能说明" });
    const dialog = document.getElementById(trigger.getAttribute("aria-controls")!) as HTMLDialogElement;
    Object.defineProperties(dialog, {
      showModal: { value: () => { dialog.open = true; } },
      close: { value: () => { dialog.open = false; fireEvent(dialog, new Event("close")); } },
    });

    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(trigger);
    const help = within(screen.getByRole("dialog", { name: "自学习如何工作" }));
    expect(help.getByText(/周围的草稿只用于在本机核对插入范围/)).toBeTruthy();
    expect(help.getByText(/发送给你配置的学习模型/)).toBeTruthy();
    const close = help.getByRole("button", { name: "关闭" });
    close.focus();
    fireEvent.click(close);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("shows the saved runtime settings while the switch has unsaved changes", async () => {
    const props = harness();
    const draft = { ...props.config, learning: { ...props.config.learning, enabled: false } };
    render(<LearningActivityPage {...props} config={draft} savedConfig={props.config} />);
    await screen.findByText("自学习运行中");
    expect((screen.getByRole("checkbox", { name: "自动学习纠错词" }) as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByText("自学习已关闭")).toBeNull();
  });

  it("shows current tasks and all history details directly, and opens the selected attempt", async () => {
    const props = harness();
    const view = render(<LearningActivityPage {...props} />);
    await screen.findByText("正在观察上屏后的修改");
    expect(screen.getByText("正在分析修改")).toBeTruthy();
    expect(screen.getByText("Kubernetes")).toBeTruthy();
    expect(screen.getByText("Learning model request timed out")).toBeTruthy();
    const history = within(screen.getByRole("region", { name: "学习历史" }));
    expect(history.getAllByRole("button", { name: /^详情/ })).toHaveLength(4);
    const routineRow = within(history.getByText("未检测到修改").closest("li")!);
    expect(routineRow.getByText("开始时间")).toBeTruthy();
    expect(routineRow.getByText("结束时间")).toBeTruthy();
    expect(routineRow.getByText("openai / learning-model")).toBeTruthy();
    expect(view.container.querySelector("details")).toBeNull();
    fireEvent.click(routineRow.getByRole("button", { name: /^详情/ }));
    expect(props.onOpenRecord).toHaveBeenCalledWith("3", "preview-activity-4");
    const current = within(screen.getByRole("region", { name: "当前任务" }));
    fireEvent.click(current.getAllByRole("button", { name: /^详情/ })[0]);
    expect(props.onOpenRecord).toHaveBeenCalledWith("1", "preview-activity-observing");
    fireEvent.click(screen.getByRole("button", { name: "管理模型" }));
    expect(props.onOpenModels).toHaveBeenCalledOnce();
  });

  it("refreshes on native changes without polling and releases event and timer resources", async () => {
    vi.useFakeTimers();
    const props = harness();
    const view = render(<LearningActivityPage {...props} />);
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); });
    expect(props.api.loadActivity).toHaveBeenCalledTimes(1);
    props.replace({ active: [], history: [], has_more: false });
    await act(async () => props.notify());
    expect(screen.getByText("当前没有学习任务")).toBeTruthy();
    expect(screen.getByText("还没有学习记录。")).toBeTruthy();
    view.unmount();
    expect(props.stop).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("discards a slow older response when the source-record filter changes", async () => {
    const props = harness();
    let resolveOld!: (value: ActivityPage) => void;
    const old = new Promise<ActivityPage>((resolve) => { resolveOld = resolve; });
    vi.mocked(props.api.loadActivity).mockImplementation(async (_limit, _offset, recordId) => {
      if (recordId === "old-record") return old;
      return { active: [], history: [], has_more: false };
    });
    const view = render(<LearningActivityPage {...props} recordId="old-record" />);
    await waitFor(() => expect(props.api.loadActivity).toHaveBeenCalledWith(30, 0, "old-record"));
    view.rerender(<LearningActivityPage {...props} recordId="new-record" />);
    await screen.findByText("还没有学习记录。");
    await act(async () => {
      resolveOld({ active: [], history: createBrowserPreviewState().learningActivity, has_more: false });
      await old;
    });
    expect(screen.queryByText("Kubernetes")).toBeNull();
    expect(screen.getByText("还没有学习记录。")).toBeTruthy();
  });

  it("loads older history independently of current tasks", async () => {
    const props = harness();
    const preview = createBrowserPreviewState("http://localhost/?preview-learning=active");
    const active = preview.learningActivity.filter((item) => item.finished_at === null);
    props.replace({ active, history: [preview.learningActivity[2]], has_more: true });
    render(<LearningActivityPage {...props} />);
    await screen.findByText("Kubernetes");
    props.replace({ active, history: [], has_more: false });
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() => expect(props.api.loadActivity).toHaveBeenLastCalledWith(30, 30, undefined));
    await screen.findByText("第 2 页");
    expect(screen.getByText("正在观察上屏后的修改")).toBeTruthy();
    expect(screen.getByText("正在分析修改")).toBeTruthy();
  });

  it("cleans up a subscription that resolves after leaving the page", async () => {
    const props = harness();
    let resolveListener!: (stop: () => void) => void;
    vi.mocked(props.api.listenUpdated).mockImplementation(() => new Promise((resolve) => { resolveListener = resolve; }));
    const view = render(<LearningActivityPage {...props} />);
    view.unmount();
    await act(async () => resolveListener(props.stop));
    expect(props.stop).toHaveBeenCalledOnce();
    expect(props.api.loadActivity).not.toHaveBeenCalled();
  });
});
