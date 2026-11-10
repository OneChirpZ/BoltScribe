import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

function openRealtimeParameters() {
  const task = within(screen.getByRole("region", { name: "实时纠错" }));
  fireEvent.click(task.getByText("模型参数"));
}

function serviceRow(name: string) {
  return within(screen.getByRole("group", { name }));
}

describe("ModelsPage fast mode", () => {
  it.each(["openai", "volc_ark"])("defaults to enabled for %s", (provider) => {
    const props = pageProps();
    props.config.llm.service_id = provider;
    render(<ModelsPage {...props} />);
    openRealtimeParameters();

    const toggle = screen.getByRole("checkbox", { name: "Fast / 低延迟" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });

  it("shows fast mode enabled for an older config without the field", () => {
    const props = pageProps();
    Reflect.deleteProperty(props.config.llm, "fast_mode_enabled");
    render(<ModelsPage {...props} text={translations["en-US"]} />);
    openRealtimeParameters();

    const toggle = screen.getByRole("checkbox", { name: "Fast / Low latency" }) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });

  it("retains an explicit disabled preference when editing the shared service", () => {
    const props = pageProps();
    const { rerender } = render(<ModelsPage {...props} />);
    openRealtimeParameters();
    fireEvent.click(screen.getByRole("checkbox", { name: "Fast / 低延迟" }));

    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed).toEqual({
      ...props.config,
      llm: { ...props.config.llm, fast_mode_enabled: false },
    });
    rerender(<ModelsPage {...props} config={changed} />);
    expect((screen.getByRole("checkbox", { name: "Fast / 低延迟" }) as HTMLInputElement).checked).toBe(false);
    fireEvent.click(serviceRow("OpenAI").getByRole("button", { name: "编辑连接" }));
    fireEvent.change(screen.getByLabelText("服务名称"), { target: { value: "Shared service" } });
    const updated: AppConfig = props.onChange.mock.calls[1][0];
    expect(updated.llm.fast_mode_enabled).toBe(false);
    expect(updated.model_services[0].name).toBe("Shared service");
  });

  it("hides fast mode for a custom provider", () => {
    const props = pageProps();
    props.config.llm.service_id = "custom";
    render(<ModelsPage {...props} />);
    openRealtimeParameters();

    expect(screen.queryByRole("checkbox", { name: "Fast / 低延迟" })).toBeNull();
    expect(screen.queryByRole("button", { name: props.text.models.fastModeHelp })).toBeNull();
  });

  it("preserves the global fast preference when changing provider or race mode", () => {
    const props = pageProps();
    props.config.llm.fast_mode_enabled = false;
    const { rerender } = render(<ModelsPage {...props} />);
    fireEvent.change(screen.getByLabelText("实时纠错 · 服务"), { target: { value: "volc_ark" } });

    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed.llm.service_id).toBe("volc_ark");
    expect(changed.llm.fast_mode_enabled).toBe(false);
    rerender(<ModelsPage {...props} config={changed} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "启用竞速模式" }));
    expect(props.onChange.mock.calls[1][0].llm.fast_mode_enabled).toBe(false);
  });
});


describe("shared model services", () => {
  it("stores a shared API key once while keeping both task profiles independent", () => {
    const props = pageProps();
    props.config.learning.llm.model = "quality-model";
    render(<ModelsPage {...props} />);
    fireEvent.click(serviceRow("OpenAI").getByRole("button", { name: "编辑连接" }));
    expect(screen.getAllByLabelText("API Key", { selector: "input" })).toHaveLength(1);
    fireEvent.change(screen.getByLabelText("API Key", { selector: "input" }), { target: { value: "rotated-shared-key" } });
    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed.model_services[0].api_key).toBe("rotated-shared-key");
    expect(changed.llm).toEqual(props.config.llm);
    expect(changed.learning).toEqual(props.config.learning);
    expect(changed.llm).not.toHaveProperty("api_key");
    expect(changed.learning.llm).not.toHaveProperty("api_key");
  });

  it("selects different models and reasoning budgets from the same service", () => {
    const props = pageProps();
    props.config.model_services[0].models = ["quality-model"];
    render(<ModelsPage {...props} />);
    fireEvent.change(screen.getByLabelText("自学习 · 模型"), { target: { value: "quality-model" } });
    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed.learning.llm.service_id).toBe(changed.llm.service_id);
    expect(changed.learning.llm.model).toBe("quality-model");
    expect(changed.learning.llm.thinking_effort).toBe("high");
    expect(changed.llm).toEqual(props.config.llm);
    expect((screen.getByLabelText("自学习 · 超时（秒）") as HTMLInputElement).max).toBe("180");
  });

  it("clears task references and race entries when a shared service is deleted", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const props = pageProps();
    render(<ModelsPage {...props} />);
    expect(screen.queryByRole("button", { name: "删除服务" })).toBeNull();
    fireEvent.click(serviceRow("OpenAI").getByRole("button", { name: "编辑连接" }));
    fireEvent.click(screen.getByRole("button", { name: "删除服务" }));
    const changed: AppConfig = props.onChange.mock.calls[0][0];
    expect(changed.model_services.some((service) => service.id === "openai")).toBe(false);
    expect(changed.llm.service_id).toBe("");
    expect(changed.learning.llm.service_id).toBe("");
    expect(changed.llm.race_targets.some((target) => target.service_id === "openai")).toBe(false);
    expect(changed.llm).not.toHaveProperty("api_key");
    confirm.mockRestore();
  });
});

describe("automatic model catalogs", () => {
  it("shows every saved service and refreshes or edits any row in one click", async () => {
    const props = pageProps();
    props.config.llm.service_id = "volc_ark";
    props.config.model_services[1].api_key = "test-ark-key";
    const fetchModels = vi.fn(async (id: string) => [id + "-remote"]);
    render(<ModelsPage {...props} savedServices={props.config.model_services} fetchModels={fetchModels} />);
    const ark = screen.getByRole("button", { name: "火山方舟", pressed: true });
    const openai = screen.getByRole("button", { name: "OpenAI", pressed: false });
    expect(screen.queryByLabelText("API Key", { selector: "input" })).toBeNull();
    await waitFor(() => expect(screen.getAllByText("1 个模型")).toHaveLength(2));
    fireEvent.click(serviceRow("OpenAI").getByRole("button", { name: "刷新模型" }));
    expect(fetchModels).toHaveBeenLastCalledWith("openai");
    expect(ark.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(openai);
    expect(ark.getAttribute("aria-pressed")).toBe("false");
    expect((screen.getByLabelText("服务名称") as HTMLInputElement).value).toBe("OpenAI");
    expect((screen.getByLabelText("API Key", { selector: "input" }) as HTMLInputElement).type).toBe("password");
    fireEvent.click(serviceRow("火山方舟").getByRole("button", { name: "编辑连接" }));
    expect((screen.getByLabelText("服务名称") as HTMLInputElement).value).toBe("火山方舟");
    expect((screen.getByLabelText("API Key", { selector: "input" }) as HTMLInputElement).type).toBe("password");
    expect(props.onChange).not.toHaveBeenCalled();
  });

  it("offers an explicit OpenAI preset when absent without changing task selections", () => {
    const props = pageProps();
    props.config.model_services = props.config.model_services.filter((service) => service.provider !== "openai");
    props.config.llm.service_id = "volc_ark";
    props.config.learning.llm.service_id = "";
    const view = render(<ModelsPage {...props} />);
    expect(screen.queryByRole("button", { name: "OpenAI" })).toBeNull();
    expect(props.onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "添加 OpenAI" }));
    const next: AppConfig = props.onChange.mock.calls[0][0];
    expect(next.model_services.slice(0, -1)).toEqual(props.config.model_services);
    expect(next.model_services[next.model_services.length - 1]).toMatchObject({
      name: "OpenAI", provider: "openai", endpoint: "https://api.openai.com/v1", api_key: "",
    });
    expect(next.llm).toEqual(props.config.llm);
    expect(next.learning).toEqual(props.config.learning);
    view.rerender(<ModelsPage {...props} config={next} />);
    expect(screen.getByRole("button", { name: "OpenAI", pressed: true })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "添加 OpenAI" })).toBeNull();
    expect(screen.getByLabelText("API Key", { selector: "input" })).toBeTruthy();
  });

  it("shares fetched model IDs across both tasks and racing without dirtying settings", async () => {
    const props = pageProps();
    props.config.model_services[1].api_key = "test-ark-key";
    const fetchModels = vi.fn(async (id: string) => [id + "-remote"]);
    render(<ModelsPage {...props} savedServices={props.config.model_services} fetchModels={fetchModels} />);
    for (const name of ["实时纠错 · 模型", "自学习 · 模型"]) {
      const select = screen.getByRole("combobox", { name });
      expect(await within(select).findByRole("option", { name: "openai-remote" })).toBeTruthy();
    }
    expect(screen.getByRole("checkbox", { name: "火山方舟 / volc_ark-remote" })).toBeTruthy();
    expect(fetchModels.mock.calls).toEqual([["openai"], ["volc_ark"]]);
    expect(props.onChange).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("自定义模型（每行一个）")).toBeNull();
    fetchModels.mockRejectedValueOnce(new Error("HTTP 403"));
    fireEvent.click(serviceRow("OpenAI").getByRole("button", {name: "刷新模型"}));
    await screen.findByText(/获取模型失败.*403/);
    expect(within(screen.getByLabelText("自学习 · 模型")).getByRole("option", {name:"openai-remote"})).toBeTruthy();
  });

  it("fetches only saved connections and discards results for a replaced key", async () => {
    const props = pageProps();
    let finishOld!: (ids: string[]) => void;
    const fetchModels = vi.fn().mockImplementationOnce(() => new Promise<string[]>((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce(["new-account-model"]);
    const original = props.config.model_services;
    const view = render(<ModelsPage {...props} savedServices={original} fetchModels={fetchModels} />);
    const next = { ...props.config, model_services: original.map((s) => s.id === "openai" ? { ...s, api_key: "replacement" } : s) };
    view.rerender(<ModelsPage {...props} config={next} savedServices={original} fetchModels={fetchModels} />);
    expect(fetchModels).toHaveBeenCalledTimes(1);
    view.rerender(<ModelsPage {...props} config={next} savedServices={next.model_services} fetchModels={fetchModels} />);
    const options = within(screen.getByLabelText("实时纠错 · 模型"));
    await options.findByRole("option", {name:"new-account-model"});
    await act(async () => finishOld(["old-account-model"]));
    expect(options.queryByRole("option", {name:"old-account-model"})).toBeNull();
    expect(fetchModels).toHaveBeenCalledTimes(2);
  });
});
