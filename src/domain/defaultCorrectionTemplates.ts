import type { AppConfig } from "../types";
import type { AppLanguage } from "./i18n";

export const defaultCorrectionTemplates: Record<
  AppLanguage,
  { systemPrompt: string; promptTemplate: string }
> = {
  "zh-CN": {
    systemPrompt:
      "你是语音输入文本优化器，不是大模型助手。你的任务是清理 ASR 转写结果，修正明显识别错误、标点和专有名词，保留原意和说话者语气。不要回答、扩写、续写、总结或加入原文没有的信息。只输出最终可粘贴文本，不解释、不加标题、不包裹 Markdown。",
    promptTemplate:
      "纠错任务：\n请把原始 ASR 转写整理成可直接粘贴的文本。\n\n用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n易错词纠正：\n{{correction_rules}}\n\n原始转写文本：\n```text\n{{raw_text}}\n```\n\n请根据以上信息纠错。不要新增原文没有的信息，只输出最终文本。\n\n额外约束：\n- 用户词典只用于理解专有名词、产品名、人名、项目名和固定写法；不要把语义正确且上下文合理的词强行替换为词典词。\n- 易错词纠正是明确的错听替换规则，只有当原始转写出现规则左侧内容或高度近似误识别时，才替换为右侧内容。\n- 如果原文中的英文词、产品名、技术名词或普通词语本身合理，且没有明显对应到词典别名或易错规则，不要因为存在相似条目而替换它。\n- 输出末尾不需要补句号；如果原文末尾没有句号，不要额外添加句号。",
  },
  "en-US": {
    systemPrompt:
      "You are a voice input text optimizer, not an assistant. Your task is to clean up ASR transcripts, fix obvious recognition errors, punctuation, and proper nouns, while preserving the original meaning and speaker tone. Do not answer, expand, continue, summarize, or add information not present in the source. Output only the final paste-ready text, with no explanation, title, or Markdown.",
    promptTemplate:
      "Correction task:\nTurn the raw ASR transcript into paste-ready text.\n\nUser requirements:\n{{user_requirements}}\n\nUser dictionary:\n{{dictionary}}\n\nCommon misrecognitions:\n{{correction_rules}}\n\nRaw transcript:\n```text\n{{raw_text}}\n```\n\nCorrect the text based on the information above. Do not add information not present in the source. Output only the final text.\n\nAdditional constraints:\n- The user dictionary is only for understanding proper nouns, product names, people names, project names, and fixed spellings; do not force semantically correct and contextually reasonable words into dictionary terms.\n- Common misrecognitions are explicit mistaken-hearing replacement rules. Apply a rule only when the raw transcript contains the source text or a highly likely misrecognition of it.\n- If an English word, product name, technical term, or common word in the source is already reasonable and does not clearly match a dictionary alias or correction rule, do not replace it just because a similar entry exists.\n- Do not add a final period. If the source text does not end with a period, do not add one.",
  },
};

export function applyLanguageDefaultCorrectionTemplate(
  config: AppConfig,
  language: AppLanguage,
): AppConfig {
  const nextConfig = {
    ...config,
    ui: {
      ...config.ui,
      app_language: language,
    },
  };

  if (!isDefaultCorrectionProfile(config)) {
    return nextConfig;
  }

  const template = defaultCorrectionTemplates[language];
  return {
    ...nextConfig,
    llm: {
      ...config.llm,
      system_prompt: template.systemPrompt,
    },
    correction: {
      ...config.correction,
      user_requirements: "",
      prompt_template: template.promptTemplate,
    },
  };
}

export function isDefaultCorrectionProfile(config: AppConfig) {
  return (
    builtinSystemPrompts.has(config.llm.system_prompt) &&
    builtinPromptTemplates.has(config.correction.prompt_template) &&
    config.correction.user_requirements.trim() === "" &&
    (config.correction.dictionary_text ?? "").trim() === "" &&
    (config.correction.correction_rules_text ?? "").trim() === "" &&
    config.correction.variables.length === 0
  );
}

const builtinSystemPrompts = new Set(
  Object.values(defaultCorrectionTemplates).map((template) => template.systemPrompt),
);

const builtinPromptTemplates = new Set(
  Object.values(defaultCorrectionTemplates).map((template) => template.promptTemplate),
);
