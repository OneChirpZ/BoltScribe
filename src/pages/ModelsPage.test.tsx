import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createBrowserPreviewState } from "../app/api/previewData";
import { translations } from "../domain/i18n";
import type { AppConfig } from "../types";
import ModelsPage from "./ModelsPage";

function pageProps(config = createBrowserPreviewState().config) {
  return {
    config,
    onChange: vi.fn(),
    onSaveConfig: vi.fn(),
    onNotice: vi.fn(),
    canSave: true,
    text: translations["zh-CN"],
  };
}

describe("ModelsPage fast mode", () => {
  it.each(["openai", "volc_ark"])("defaults to enabled for %s", (provider) => {
    const props = pageProps();
    props.config.llm.provider = provider;
    render(<ModelsPage {...props} />);

    const toggle = screen.getByRole("checkbox", { name: "Fast / 低延迟" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });

  it("shows fast mode enabled for an older config without the field", () => {
    const props = pageProps();
    Reflect.deleteProperty(props.config.llm, "fast_mode_enabled");
    render(<ModelsPage {...props} text={translations["en-US"]} />);

    const toggle = screen.getByRole("checkbox", { name: "Fast / Low latency" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });

  it("retains an explicit disabled preference when saving a model preset", () => {
    const props = pageProps();
    const { rerender } = render(<ModelsPage {...props} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Fast / 低延迟" }));

    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed).toEqual({
      ...props.config,
      llm: { ...props.config.llm, fast_mode_enabled: false },
    });
    rerender(<ModelsPage {...props} config={changed} />);
    expect((screen.getByRole("checkbox", { name: "Fast / 低延迟" }) as HTMLInputElement).checked).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "保存预设" }));
    expect(props.onSaveConfig.mock.calls[0][0].llm.fast_mode_enabled).toBe(false);
  });

  it("hides fast mode for a custom provider", () => {
    const props = pageProps();
    props.config.llm.provider = "custom";
    render(<ModelsPage {...props} />);

    expect(screen.queryByRole("checkbox", { name: "Fast / 低延迟" })).toBeNull();
    expect(screen.queryByRole("button", { name: props.text.models.fastModeHelp })).toBeNull();
  });

  it("preserves the global fast preference when changing provider or race mode", () => {
    const props = pageProps();
    props.config.llm.fast_mode_enabled = false;
    const { rerender } = render(<ModelsPage {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "火山方舟" }));

    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed.llm.provider).toBe("volc_ark");
    expect(changed.llm.fast_mode_enabled).toBe(false);
    rerender(<ModelsPage {...props} config={changed} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "启用竞速模式" }));
    expect(props.onChange.mock.calls[1][0].llm.fast_mode_enabled).toBe(false);
  });
});
