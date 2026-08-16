import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import { translations } from "../domain/i18n";
import MainApp from "./MainApp";
import { useMainAppController } from "./controllers/useMainAppController";

vi.mock("./controllers/useMainAppController", () => ({
  useMainAppController: vi.fn(),
}));

describe("MainApp correction navigation", () => {
  it("keeps every LLM correction subpage visible outside the correction page", () => {
    const requestCorrectionSection = vi.fn();
    vi.mocked(useMainAppController).mockReturnValue({
      config: {
        state: {
          config: null,
          correctionSection: "requirements",
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
      actions: {},
    } as unknown as ReturnType<typeof useMainAppController>);

    render(<MainApp />);

    for (const label of ["纠错要求", "用户词典", "易错词纠正", "Prompt 模板"]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
    expect(screen.getByRole("navigation", { name: "LLM 纠错设置" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "用户词典" }));
    expect(requestCorrectionSection).toHaveBeenCalledWith("dictionary");
  });
});
