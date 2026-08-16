import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { updateAsrConnectionRetry } from "../../domain/asrConnectionRetry";
import { translations } from "../../domain/i18n";
import { createBrowserPreviewState } from "../../app/api/previewData";
import AdvancedSettingsSection from "./AdvancedSettingsSection";

const idleVadStatus = {
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

function advancedProps(
  config = createBrowserPreviewState().config,
  overrides: Partial<ComponentProps<typeof AdvancedSettingsSection>> = {},
): ComponentProps<typeof AdvancedSettingsSection> {
  return {
    config,
    onChange: vi.fn(),
    dataDir: {
      path: "/tmp/BoltScribe",
      default_path: "/tmp/BoltScribe",
      is_default: true,
      cleanup_warning: null,
    },
    canSave: true,
    canChangeDataDir: true,
    onOpenDataDir: vi.fn(),
    onChooseDataDir: vi.fn(),
    onResetDataDir: vi.fn(),
    vadTestStatus: idleVadStatus,
    onStartVadTest: vi.fn(),
    onUpdateVadTestSettings: vi.fn(),
    onStopVadTest: vi.fn(),
    text: translations["zh-CN"],
    ...overrides,
  };
}

describe("AdvancedSettingsSection", () => {
  it("starts collapsed and exposes the retry controls when opened", async () => {
    const config = createBrowserPreviewState("http://localhost/?preview-language=zh-CN").config;
    const { container } = render(
      <AdvancedSettingsSection {...advancedProps(config)} />,
    );
    const details = container.querySelector("details");

    expect(details?.hasAttribute("open")).toBe(false);
    fireEvent.click(screen.getByText("高级设置"));
    expect((details as HTMLDetailsElement).open).toBe(true);
    const liveAttempts = await screen.findByLabelText("停止录音后的实时连接尝试次数");
    expect(liveAttempts.getAttribute("min")).toBe("1");
    expect(liveAttempts.getAttribute("max")).toBe("20");
    expect((liveAttempts as HTMLInputElement).value).toBe("3");
    const liveTimeout = screen.getByLabelText("实时识别单次连接超时");
    expect(liveTimeout.getAttribute("min")).toBe("500");
    expect(liveTimeout.getAttribute("max")).toBe("10000");
    expect((liveTimeout as HTMLInputElement).value).toBe("3000");
    expect(screen.getByLabelText("录音文件识别连接尝试次数").getAttribute("max")).toBe("10");
    const timeout = screen.getByLabelText("录音文件识别单次连接超时");
    expect(timeout.getAttribute("max")).toBe("60000");
    expect((timeout as HTMLInputElement).value).toBe("10000");
  });

  it("writes edited values through the nested ASR retry config", async () => {
    const config = createBrowserPreviewState("http://localhost/?preview-language=en-US").config;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection {...advancedProps(config, { onChange, text: translations["en-US"] })} />,
    );

    fireEvent.click(screen.getByText("Advanced Settings"));
    const input = await screen.findByLabelText("Live attempts after recording stops");
    fireEvent.change(input, { target: { value: "9" } });
    fireEvent.blur(input);

    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange.mock.calls[0][0].asr.connection_retry).toEqual({
      ...config.asr.connection_retry,
      live_post_stop_max_attempts: 9,
    });
  });

  it("clamps numeric limits and keeps the maximum delay above the initial delay", () => {
    const config = createBrowserPreviewState().config;

    const attempts = updateAsrConnectionRetry(config, "live_post_stop_max_attempts", "99");
    const liveTimeout = updateAsrConnectionRetry(config, "live_connect_timeout_ms", "100");
    const delays = updateAsrConnectionRetry(config, "initial_delay_ms", "8000");

    expect(attempts.asr.connection_retry.live_post_stop_max_attempts).toBe(20);
    expect(liveTimeout.asr.connection_retry.live_connect_timeout_ms).toBe(500);
    expect(delays.asr.connection_retry.initial_delay_ms).toBe(8000);
    expect(delays.asr.connection_retry.max_delay_ms).toBe(8000);
  });

  it("keeps intermediate numeric drafts editable and commits on blur", async () => {
    const config = createBrowserPreviewState().config;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection {...advancedProps(config, { onChange })} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("录音文件识别单次连接超时") as HTMLInputElement;

    fireEvent.focus(input);
    for (const draft of ["", "8", "80", "800"]) {
      fireEvent.change(input, { target: { value: draft } });
    }

    expect(input.value).toBe("800");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: "8000" } });
    expect(input.value).toBe("8000");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.blur(input);
    expect(onChange.mock.calls[0][0].asr.connection_retry.connect_timeout_ms).toBe(8000);
  });

  it("restores the configured value when an empty numeric draft loses focus", async () => {
    const config = createBrowserPreviewState().config;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection {...advancedProps(config, { onChange })} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("录音文件识别单次连接超时") as HTMLInputElement;

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);

    expect(input.value).toBe("10000");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows the cross-field normalized value after committing a lower maximum delay", async () => {
    const config = createBrowserPreviewState().config;
    config.asr.connection_retry.initial_delay_ms = 8000;
    config.asr.connection_retry.max_delay_ms = 8000;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection {...advancedProps(config, { onChange })} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("最大重试等待") as HTMLInputElement;

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "100" } });
    fireEvent.blur(input);

    expect(input.value).toBe("8000");
    expect(onChange.mock.calls[0][0].asr.connection_retry.max_delay_ms).toBe(8000);
  });

  it("keeps VAD protection and data directory inside the collapsed advanced area", async () => {
    const startVadTest = vi.fn();
    const openDataDir = vi.fn();
    const { container } = render(
      <AdvancedSettingsSection
        {...advancedProps(undefined, {
          onStartVadTest: startVadTest,
          onOpenDataDir: openDataDir,
        })}
      />,
    );

    expect(screen.queryByText("无人声保护")).toBeNull();
    expect(screen.queryByText("数据目录")).toBeNull();

    fireEvent.click(screen.getByText("高级设置"));
    expect(await screen.findByText("无人声保护")).toBeTruthy();
    expect(screen.getByText("数据目录")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "测试麦克风" }));
    fireEvent.click(screen.getByRole("button", { name: "打开数据目录" }));
    expect(startVadTest).toHaveBeenCalledOnce();
    expect(openDataDir).toHaveBeenCalledOnce();
    expect(container.querySelector(".advanced-settings-content .vad-protection-card")).toBeTruthy();
    expect(container.querySelector(".advanced-settings-content .data-dir-panel")).toBeTruthy();
  });

  it("stops an active VAD test exactly once when advanced settings collapse", async () => {
    const stopVadTest = vi.fn();
    const { container } = render(
      <AdvancedSettingsSection
        {...advancedProps(undefined, {
          vadTestStatus: {
            ...idleVadStatus,
            mode: "listening",
            revision: 1,
          },
          onStopVadTest: stopVadTest,
        })}
      />,
    );

    const summary = screen.getByText("高级设置");
    fireEvent.click(summary);
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(true);
    await screen.findByText("无人声保护");

    fireEvent.click(summary);

    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(false);
    await waitFor(() => expect(stopVadTest).toHaveBeenCalledOnce());
    expect(screen.queryByText("无人声保护")).toBeNull();
  });
});
