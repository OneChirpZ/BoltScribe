import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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

function dispatchPointerDown(target: Element, button: number) {
  const event = new Event("pointerdown", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "button", { value: button });
  fireEvent(target, event);
}
