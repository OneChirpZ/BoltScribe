import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { StrictMode, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createBrowserPreviewState } from "../app/api/previewData";
import LearningSourceDialog from "./LearningSourceDialog";

const originalShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
const originalClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");

beforeEach(() => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: function (this: HTMLDialogElement) { this.setAttribute("open", ""); },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute("open");
      // Browsers dispatch close after close() returns, including effect cleanup.
      queueMicrotask(() => this.dispatchEvent(new Event("close")));
    },
  });
});

afterEach(() => {
  if (originalShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", originalShowModal);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
  if (originalClose) Object.defineProperty(HTMLDialogElement.prototype, "close", originalClose);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "close");
});

describe("LearningSourceDialog", () => {
  it("stays open through StrictMode effect replay with all four stages visible", async () => {
    const onClose = vi.fn();
    const preview = createBrowserPreviewState();
    const record = {
      ...preview.history[0],
      raw_text: "用库伯内特斯演示",
      corrected_text: "请用库伯内特斯演示。",
    };
    const activity = {
      ...preview.learningActivity[0],
      edited_text: "请用 Kubernetes 演示。",
      model_output: '{\n  "terms": ["Kubernetes"]\n}',
    };
    function Source() {
      const [visible, setVisible] = useState(true);
      return visible ? <LearningSourceDialog record={record} activity={activity} language="zh-CN" onClose={() => {
        onClose();
        setVisible(false);
      }} /> : null;
    }

    render(<StrictMode><Source /></StrictMode>);
    await act(async () => { await Promise.resolve(); });
    const dialog = screen.getByRole("dialog", { name: "来源语音" }) as HTMLDialogElement;
    expect(dialog.open).toBe(true);
    expect(onClose).not.toHaveBeenCalled();
    const content = within(screen.getByRole("region", { name: "来源语音" }));
    expect(content.getAllByRole("heading", { level: 2 }).map((heading) => heading.textContent)).toEqual([
      "原始转写", "纠错后输出", "用户修改后", "学习模型输出",
    ]);
    expect(content.getByText(record.raw_text)).toBeTruthy();
    expect(content.getByText(record.corrected_text)).toBeTruthy();
    expect(content.getByText(activity.edited_text)).toBeTruthy();
    expect(dialog.querySelector("pre")?.textContent).toBe(activity.model_output);
    expect(content.getByText("Kubernetes", { selector: "strong" })).toBeTruthy();
    expect(dialog.querySelector("details")).toBeNull();
    fireEvent(dialog, new Event("close"));
    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows unrecorded stages and keeps the saved terms for older activities", () => {
    const preview = createBrowserPreviewState();
    const activity = { ...preview.learningActivity[0], edited_text: null, model_output: null };
    render(<LearningSourceDialog record={preview.history[0]} activity={activity} language="en-US" onClose={vi.fn()} />);

    const dialog = within(screen.getByRole("dialog", { name: "Source dictation" }));
    expect(dialog.getByRole("heading", { name: "User-edited text" })).toBeTruthy();
    expect(dialog.getByRole("heading", { name: "Learning model output" })).toBeTruthy();
    expect(dialog.getAllByText("Not recorded")).toHaveLength(2);
    expect(dialog.getByText("Kubernetes", { selector: "strong" })).toBeTruthy();
  });
});
