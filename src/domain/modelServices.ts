import type { AppConfig, ModelService } from "../types";
import { providerPresets } from "./providers";

export function defaultModelServices(): ModelService[] {
  return Object.entries(providerPresets).map(([provider, preset]) => ({
    id: provider, name: preset.label, provider, endpoint: preset.endpoint,
    api_format: preset.api_format, api_key: "", models: [],
  }));
}

export function modelService(config: AppConfig, id: string) {
  return config.model_services.find((service) => service.id === id);
}

export function serviceModels(config: AppConfig, id: string, fetched?: string[]): string[] {
  const service = modelService(config, id);
  if (!service) return [];
  const selected = [config.llm, config.learning.llm]
    .filter((profile) => profile.service_id === id).map((profile) => profile.model);
  const raced = config.llm.race_targets.filter((target) => target.service_id === id).map((target) => target.model);
  return Array.from(new Set([...(fetched ?? []), ...service.models, ...selected, ...raced]
    .map((model) => model.trim()).filter(Boolean)));
}

export function removeModelService(config: AppConfig, id: string): AppConfig {
  return {
    ...config,
    model_services: config.model_services.filter((service) => service.id !== id),
    llm: {
      ...config.llm,
      service_id: config.llm.service_id === id ? "" : config.llm.service_id,
      race_targets: config.llm.race_targets.filter((target) => target.service_id !== id),
      race_models: [],
    },
    learning: { ...config.learning, llm: {
      ...config.learning.llm,
      service_id: config.learning.llm.service_id === id ? "" : config.learning.llm.service_id,
    } },
  };
}

export function modelManagerText(language: string) {
  return language === "en-US" ? english : chinese;
}

const chinese = {
  services: "模型服务", service: "服务", serviceName: "服务名称", provider: "服务商",
  add: "添加服务", remove: "删除服务", removeConfirm: "删除这个模型服务？使用它的用途需要重新选择服务。",
  addOpenAI: "添加 OpenAI", editSettings: "编辑连接", closeSettings: "收起连接",
  endpoint: "API 地址", apiKey: "API Key", format: "API 格式", catalog: "自定义模型（每行一个）",
  empty: "添加模型服务后，可供实时纠错和自学习共同使用。", newService: "新模型服务",
  sharedHelp: "API 地址和密钥统一保存在这里，供各用途选择。",
  connectionSettings: "连接设置", refreshModels: "刷新模型", fetching: "正在获取模型…",
  saveToFetch: "保存服务连接后自动获取模型。", fetchError: "获取模型失败", chooseModel: "选择模型",
  otherModel: "输入其他模型 ID…", modelCount: (n: number) => `${n} 个模型`,
  realtime: "实时纠错", learning: "自学习", model: "模型", choose: "请选择服务",
  thinking: "启用思考", effort: "思考强度", timeout: "超时（秒）", maxTokens: "最大输出 Token",
  advanced: "模型参数", configure: "管理模型", unconfigured: "尚未配置",
};

const english: typeof chinese = {
  services: "Model services", service: "Service", serviceName: "Service name", provider: "Provider",
  add: "Add service", remove: "Delete service", removeConfirm: "Delete this model service? Tasks using it will need another service.",
  addOpenAI: "Add OpenAI", editSettings: "Edit connection", closeSettings: "Close editor",
  endpoint: "API endpoint", apiKey: "API key", format: "API format", catalog: "Custom models (one per line)",
  empty: "Add a model service for realtime correction and self-learning to share.", newService: "New model service",
  sharedHelp: "Manage API endpoints and keys here, then select them for each task.",
  connectionSettings: "Connection settings", refreshModels: "Refresh models", fetching: "Fetching models…",
  saveToFetch: "Save the service connection to fetch models.", fetchError: "Could not fetch models", chooseModel: "Select a model",
  otherModel: "Enter another model ID…", modelCount: (n: number) => `${n} models`,
  realtime: "Realtime correction", learning: "Self-learning", model: "Model", choose: "Select a service",
  thinking: "Enable thinking", effort: "Reasoning effort", timeout: "Timeout (seconds)", maxTokens: "Maximum output tokens",
  advanced: "Model parameters", configure: "Manage models", unconfigured: "Not configured",
};
