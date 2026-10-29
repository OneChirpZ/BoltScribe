import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createBrowserPreviewState } from "../../app/api/previewData";
import { translations } from "../../domain/i18n";
import type { AppConfig } from "../../types";
import AppearanceSystemSection from "./AppearanceSystemSection";

afterEach(() => {
  vi.unstubAllGlobals();
});

function usePlatform(platform: string) {
  vi.stubGlobal("navigator", { platform, userAgent: "" });
}

describe("AppearanceSystemSection", () => {
  it("lets macOS users hide and restore the menu bar icon without changing Dock visibility", () => {
    usePlatform("MacIntel");
    const config = createBrowserPreviewState().config;
    config.system.hide_dock_icon = true;
    const onChange = vi.fn();
    const text = translations["zh-CN"];
    const { rerender } = render(
      <AppearanceSystemSection config={config} onChange={onChange} text={text} />,
    );
    const toggle = screen.getByRole("checkbox", { name: "显示菜单栏图标" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);

    fireEvent.click(toggle);
    const hiddenConfig: AppConfig = onChange.mock.calls[0][0];
    expect(hiddenConfig).toEqual({
      ...config,
      system: { ...config.system, show_menu_bar_icon: false },
    });
    rerender(<AppearanceSystemSection config={hiddenConfig} onChange={onChange} text={text} />);
    expect(toggle.checked).toBe(false);

    fireEvent.click(toggle);
    expect(onChange.mock.calls[1][0]).toEqual(config);
  });

  it("shows the icon as enabled when an older config has no visibility preference", () => {
    usePlatform("MacIntel");
    const config = createBrowserPreviewState().config;
    Reflect.deleteProperty(config.system, "show_menu_bar_icon");
    render(
      <AppearanceSystemSection config={config} onChange={vi.fn()} text={translations["en-US"]} />,
    );

    const toggle = screen.getByRole("checkbox", { name: "Show menu bar icon" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });

  it.each(["Win32", "Linux x86_64"])("omits macOS visibility controls on %s", (platform) => {
    usePlatform(platform);
    render(
      <AppearanceSystemSection
        config={createBrowserPreviewState().config}
        onChange={vi.fn()}
        text={translations["en-US"]}
      />,
    );

    expect(screen.queryByRole("checkbox", { name: "Show menu bar icon" })).toBeNull();
    expect(screen.queryByRole("checkbox", { name: "Hide in macOS Dock" })).toBeNull();
    expect(screen.getByRole("checkbox", { name: "Launch at login" })).toBeTruthy();
  });
});
