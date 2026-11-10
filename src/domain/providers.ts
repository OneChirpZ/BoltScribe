export const providerPresets = {
  openai: {
    label: "OpenAI",
    endpoint: "https://api.openai.com/v1",
    api_format: "responses",
    models: ["gpt-5.4-mini", "gpt-5.4", "gpt-4.1-mini", "gpt-6-luna"],
  },
  volc_ark: {
    label: "火山方舟",
    endpoint: "https://ark.cn-beijing.volces.com/api/v3",
    api_format: "responses",
    models: ["doubao-seed-2-0-lite-260428", "doubao-seed-2-0-mini-260428", "doubao-seed-2-1-lite-260915", "doubao-seed-2-1-turbo-260628"],
  },
  custom: {
    label: "自定义",
    endpoint: "",
    api_format: "chat_completions",
    models: [],
  },
} as const;

export const thinkingEfforts = ["none", "minimal", "low", "medium", "high", "xhigh"];
