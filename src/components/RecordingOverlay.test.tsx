import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { translations } from "../domain/i18n";
import type { WorkflowStatus } from "../types";
import RecordingOverlay from "./RecordingOverlay";

const recordingStatus: WorkflowStatus = {
  mode: "recording",
  stage: "recording",
  code: "recording",
  detail: null,
  message: "recording",
  current_audio_path: null,
  last_record_id: null,
  revision: 1,
};

afterEach(() => {
  vi.useRealTimers();
});

describe("RecordingOverlay dragging", () => {
  it("starts only from a primary press outside the cancel button", () => {
    const onDragStart = vi.fn();
    const onCancel = vi.fn();
    render(
      <RecordingOverlay
        status={recordingStatus}
        audioLevel={0}
        audioLevelRevision={1}
        audioLevelSequence={0}
        baseWidth={340}
        scale={0.5}
        onCancel={onCancel}
        onDragStart={onDragStart}
        text={translations["zh-CN"]}
      />,
    );

    dispatchPointerDown(screen.getByRole("status"), 0);
    dispatchPointerDown(screen.getByRole("status"), 2);
    const cancel = screen.getByRole("button", { name: translations["zh-CN"].overlay.cancel });
    dispatchPointerDown(cancel, 0);
    fireEvent.click(cancel);

    expect(onDragStart).toHaveBeenCalledTimes(1);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe("RecordingOverlay state motion", () => {
  it("keeps a screen-space edge and marks the active listening stage", () => {
    const { container } = render(
      <RecordingOverlay
        status={recordingStatus}
        audioLevel={0}
        audioLevelRevision={1}
        audioLevelSequence={0}
        baseWidth={340}
        scale={0.5}
        text={translations["zh-CN"]}
      />,
    );

    const overlay = container.querySelector<HTMLElement>(".recording-overlay");
    expect(overlay).not.toBeNull();
    expect(overlay?.classList.contains("overlay-stage-recording")).toBe(true);
    expect(overlay?.style.getPropertyValue("--recording-overlay-edge-width")).toBe("2px");
    expect(overlay?.style.getPropertyValue("--recording-overlay-effect-margin")).toBe("12px");
  });

  it("crossfades the outgoing and incoming labels before removing the old one", () => {
    vi.useFakeTimers();
    const recognizingStatus: WorkflowStatus = {
      ...recordingStatus,
      mode: "processing",
      stage: "recognizing",
      code: "recognizing",
      message: "recognizing",
      revision: 2,
    };
    const { container, rerender } = render(
      <RecordingOverlay
        status={recordingStatus}
        audioLevel={0}
        audioLevelRevision={1}
        audioLevelSequence={0}
        baseWidth={340}
        scale={0.5}
        text={translations["zh-CN"]}
      />,
    );

    rerender(
      <RecordingOverlay
        status={recognizingStatus}
        audioLevel={0}
        audioLevelRevision={2}
        audioLevelSequence={0}
        baseWidth={340}
        scale={0.5}
        text={translations["zh-CN"]}
      />,
    );

    expect(container.querySelector(".overlay-label-previous")?.textContent).toBe("正在听取");
    expect(container.querySelector(".overlay-label-current")?.textContent).toBe("ASR 识别中");

    act(() => vi.advanceTimersByTime(240));

    expect(container.querySelector(".overlay-label-previous")).toBeNull();
    expect(container.querySelector(".overlay-label-current")?.textContent).toBe("ASR 识别中");
  });
});

function dispatchPointerDown(target: Element, button: number) {
  const event = new Event("pointerdown", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "button", { value: button });
  fireEvent(target, event);
}
