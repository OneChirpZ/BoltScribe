import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { updateAsrConnectionRetry } from "../../domain/asrConnectionRetry";
import { translations } from "../../domain/i18n";
import { createBrowserPreviewState } from "../../app/api/previewData";
import AdvancedSettingsSection from "./AdvancedSettingsSection";

describe("AdvancedSettingsSection", () => {
  it("starts collapsed and exposes the retry controls when opened", async () => {
    const config = createBrowserPreviewState("http://localhost/?preview-language=zh-CN").config;
    const { container } = render(
      <AdvancedSettingsSection config={config} onChange={vi.fn()} text={translations["zh-CN"]} />,
    );
    const details = container.querySelector("details");

    expect(details?.hasAttribute("open")).toBe(false);
    fireEvent.click(screen.getByText("高级设置"));
    expect((details as HTMLDetailsElement).open).toBe(true);
    const liveAttempts = await screen.findByLabelText("停止录音后的实时连接尝试次数");
    expect(liveAttempts.getAttribute("min")).toBe("1");
    expect(liveAttempts.getAttribute("max")).toBe("20");
    expect((liveAttempts as HTMLInputElement).value).toBe("3");
    expect(screen.getByLabelText("录音文件识别连接尝试次数").getAttribute("max")).toBe("10");
    const timeout = screen.getByLabelText("单次连接超时");
    expect(timeout.getAttribute("max")).toBe("60000");
    expect((timeout as HTMLInputElement).value).toBe("10000");
  });

  it("writes edited values through the nested ASR retry config", async () => {
    const config = createBrowserPreviewState("http://localhost/?preview-language=en-US").config;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection config={config} onChange={onChange} text={translations["en-US"]} />,
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
    const delays = updateAsrConnectionRetry(config, "initial_delay_ms", "8000");

    expect(attempts.asr.connection_retry.live_post_stop_max_attempts).toBe(20);
    expect(delays.asr.connection_retry.initial_delay_ms).toBe(8000);
    expect(delays.asr.connection_retry.max_delay_ms).toBe(8000);
  });

  it("keeps intermediate numeric drafts editable and commits on blur", async () => {
    const config = createBrowserPreviewState().config;
    const onChange = vi.fn();
    render(
      <AdvancedSettingsSection config={config} onChange={onChange} text={translations["zh-CN"]} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("单次连接超时") as HTMLInputElement;

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
      <AdvancedSettingsSection config={config} onChange={onChange} text={translations["zh-CN"]} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("单次连接超时") as HTMLInputElement;

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
      <AdvancedSettingsSection config={config} onChange={onChange} text={translations["zh-CN"]} />,
    );
    fireEvent.click(screen.getByText("高级设置"));
    const input = await screen.findByLabelText("最大重试等待") as HTMLInputElement;

    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "100" } });
    fireEvent.blur(input);

    expect(input.value).toBe("8000");
    expect(onChange.mock.calls[0][0].asr.connection_retry.max_delay_ms).toBe(8000);
  });
});
