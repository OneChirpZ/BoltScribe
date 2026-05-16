import type { ModelPreset } from "../types";

export const providerPresets = {
  openai: {
    label: "OpenAI",
    endpoint: "https://api.openai.com/v1",
    api_format: "responses",
    models: ["gpt-5.4-mini", "gpt-5.4", "gpt-4.1-mini"],
  },
  volc_ark: {
    label: "火山方舟",
    endpoint: "https://ark.cn-beijing.volces.com/api/v3",
    api_format: "responses",
    models: ["doubao-seed-2-0-lite-260428", "doubao-seed-2-0-mini-260428"],
  },
  custom: {
    label: "自定义",
    endpoint: "",
    api_format: "chat_completions",
    models: [],
  },
} as const;

export const thinkingEfforts = ["none", "minimal", "low", "medium", "high", "xhigh"];

export function providerLabel(provider: string) {
  const preset = providerPresets[provider as keyof typeof providerPresets];
  return preset?.label ?? provider;
}

export function modelsForProvider(provider: string, customPresets: ModelPreset[] = []) {
  const builtInModels = providerPresets[provider as keyof typeof providerPresets]?.models ?? [];
  const customModels = customPresets
    .filter((preset) => preset.provider === provider)
    .map((preset) => preset.model.trim())
    .filter(Boolean);
  return Array.from(new Set([...builtInModels, ...customModels]));
}

export function builtInModelsForProvider(provider: string): string[] {
  return [...(providerPresets[provider as keyof typeof providerPresets]?.models ?? [])];
}

export function saveModelPreset(presets: ModelPreset[], provider: string, previousModel: string, nextModel: string) {
  const providerKey = provider.trim();
  const model = nextModel.trim();
  const previous = previousModel.trim();
  if (!providerKey || !model) {
    return presets;
  }

  const next = presets
    .filter((preset) => preset.provider.trim() && preset.model.trim())
    .filter((preset) => !(preset.provider === providerKey && preset.model === previous))
    .filter((preset) => !(preset.provider === providerKey && preset.model === model));
  return [...next, { provider: providerKey, model }];
}

export function deleteModelPreset(presets: ModelPreset[], provider: string, model: string) {
  const providerKey = provider.trim();
  const modelKey = model.trim();
  if (!providerKey || !modelKey) {
    return presets;
  }

  return presets.filter((preset) => !(preset.provider === providerKey && preset.model === modelKey));
}
