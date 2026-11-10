import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { translations } from "../domain/i18n";
import { appApi } from "./api";
import { createBrowserPreviewState } from "./api/previewData";
import type { HistoryRecord, LearningActivity } from "../types";
import MainApp from "./MainApp";
import { useMainAppController } from "./controllers/useMainAppController";

vi.mock("./controllers/useMainAppController", () => ({
  useMainAppController: vi.fn(),
}));

vi.mock("../pages/LearningActivityPage", () => ({
  default: ({ onOpenRecord }: { onOpenRecord: (id: string, activityId: string) => void }) => <button type="button" onClick={() => onOpenRecord("1", "preview-activity-1")}>打开测试来源</button>,
}));

vi.mock("../components/LearningSourceDialog", () => ({
  default: ({ activity }: { activity?: LearningActivity }) => <div role="dialog">{activity?.model_output}</div>,
}));

afterEach(() => { vi.restoreAllMocks(); });

function mockLearningPage(config: ReturnType<typeof createBrowserPreviewState>["config"]) {
  const state = {
    config: {
      state: { config, correctionSection: "learning", dirty: false, language: "zh-CN", page: "correction", pending: null, saving: false, text: translations["zh-CN"] },
      refs: { cancelButtonRef: createRef<HTMLButtonElement>(), contentScrollRef: createRef<HTMLDivElement>(), dialogRef: createRef<HTMLElement>() },
      actions: { change: vi.fn(), requestPageChange: vi.fn(), requestCorrectionSection: vi.fn() },
    },
    history: { state: { stats: null, records: [], pageIndex: 0, hasOlder: false }, actions: {} },
    learning: { active: [], error: null, recordId: undefined },
    audio: { state: {}, actions: {} },
    permissions: { state: { accessibilityGranted: true, guideVisible: false, needsAccessibility: false }, actions: {} },
    system: { state: { appVersion: null, dataDir: "/preview/first" }, actions: { openAppDir: vi.fn(), openGitHub: vi.fn() } },
    workflow: {},
    notice: { current: null, show: vi.fn() },
    view: { busy: false, showSaveBar: false },
    actions: { openLearning: vi.fn() },
  };
  vi.mocked(useMainAppController).mockImplementation(() => state as unknown as ReturnType<typeof useMainAppController>);
  return state;
}

describe("MainApp correction navigation", () => {
  it("keeps every LLM correction subpage visible outside the correction page", () => {
    const requestCorrectionSection = vi.fn();
    vi.mocked(useMainAppController).mockReturnValue({
      config: {
        state: {
          config: null,
          correctionSection: "prompt",
          dirty: false,
          language: "zh-CN",
          page: "home",
          pending: null,
          saving: false,
          text: translations["zh-CN"],
        },
        refs: {
          cancelButtonRef: createRef<HTMLButtonElement>(),
          contentScrollRef: createRef<HTMLDivElement>(),
          dialogRef: createRef<HTMLElement>(),
        },
        actions: {
          change: vi.fn(),
          requestPageChange: vi.fn(),
          requestCorrectionSection,
        },
      },
      history: { state: { stats: null } },
      learning: { active: [], error: null },
      audio: { state: {} },
      permissions: {
        state: {
          accessibilityGranted: true,
          guideVisible: false,
          needsAccessibility: false,
        },
      },
      system: {
        state: { appVersion: null },
        actions: { openAppDir: vi.fn(), openGitHub: vi.fn() },
      },
      workflow: {},
      notice: { current: null },
      view: { busy: false, showSaveBar: false },
      actions: { openLearning: () => requestCorrectionSection("learning") },
    } as unknown as ReturnType<typeof useMainAppController>);

    render(<MainApp />);

    for (const label of ["用户词典", "易错词纠正", "Prompt 模板"]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
    expect(screen.getByRole("navigation", { name: "LLM 纠错设置" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "用户词典" }));
    expect(requestCorrectionSection).toHaveBeenCalledWith("dictionary");
    fireEvent.click(screen.getByRole("button", { name: "自学习 Beta" }));
    expect(requestCorrectionSection).toHaveBeenCalledWith("learning");
    expect(screen.queryByRole("button", { name: "纠错要求" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Prompt 模板" }));
    expect(requestCorrectionSection).toHaveBeenCalledWith("prompt");
  });

  it.each(["page", "section", "directory"])("discards a delayed source lookup after the %s changes", async (change) => {
    const preview = createBrowserPreviewState();
    let resolveSource!: (record: HistoryRecord | null) => void;
    vi.spyOn(appApi.history, "loadRecord").mockImplementation(() => new Promise((resolve) => { resolveSource = resolve; }));
    const state = mockLearningPage(preview.config);
    const view = render(<MainApp />);
    fireEvent.click(screen.getByRole("button", { name: "打开测试来源" }));
    expect(appApi.history.loadRecord).toHaveBeenCalledWith("1");
    if (change === "page") state.config.state.page = "history";
    else if (change === "section") state.config.state.correctionSection = "rules";
    else state.system.state.dataDir = "/preview/second";
    view.rerender(<MainApp />);
    await act(async () => resolveSource(preview.history[0]));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(state.notice.show).not.toHaveBeenCalled();
  });

  it("shows the selected attempt's model output when it arrives while the source is open", async () => {
    const preview = createBrowserPreviewState();
    mockLearningPage(preview.config);
    const finished = preview.learningActivity[0];
    let current: LearningActivity = { ...finished, finished_at: null, state: "analyzing", model_output: null };
    let updated = () => {};
    const stop = vi.fn();
    vi.spyOn(appApi.history, "loadRecord").mockResolvedValue(preview.history[0]);
    const loadSource = vi.spyOn(appApi.learning, "loadSource").mockImplementation(async () => current);
    vi.spyOn(appApi.learning, "listenUpdated").mockImplementation(async (handler) => {
      updated = handler;
      return stop;
    });
    render(<MainApp />);
    fireEvent.click(screen.getByRole("button", { name: "打开测试来源" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(loadSource).toHaveBeenCalledTimes(2));
    expect(loadSource).toHaveBeenLastCalledWith("1", "preview-activity-1");
    current = finished;
    await act(async () => updated());
    expect(screen.getByRole("dialog").textContent).toBe(finished.model_output);
    expect(stop).toHaveBeenCalledOnce();
  });
});
