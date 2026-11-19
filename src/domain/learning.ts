import type { LearningConfig, LearningStatus } from "../types";

export function defaultLearningConfig(): LearningConfig {
  return {
    enabled: false,
    llm: {
      service_id: "openai",
      model: "",
      race_enabled: false,
      race_models: [],
      race_targets: [],
      system_prompt: "",
      temperature: 0.2,
      timeout_secs: 60,
      fast_mode_enabled: false,
      thinking_enabled: true,
      thinking_effort: "high",
      max_output_tokens: 8192,
    },
  };
}

export function emptyLearningStatus(): LearningStatus {
  return {
    observation: { supported: true, active: false, notifications: 0, text_reads: 0, completed: 0, skipped: 0 },
    processing: false, pending: false, submitted: 0, analyzed: 0,
    accepted: 0, rejected: 0, skipped: 0, last_error: null,
  };
}

export function learningText(language: string) {
  return language === "en-US" ? english : chinese;
}

const chinese = {
  title: "自学习",
  description: "从语音上屏后的手动修改中学习专有词和固定写法。",
  enabled: "自动学习纠错词",
  enabledHelp: "开启后，学习模型会分析相关转写和手动修改，并参考现有词典和易错规则。",
  modelRequired: "配置学习模型后自动生效。",
  unsupported: "当前平台暂不支持自动观察输入；已学习词条仍可管理。",
  modelTitle: "学习模型",
  terms: "自学习词条", rules: "自学习规则", search: "搜索自学习词条", searchRules: "搜索自学习规则",
  empty: "还没有自学习词条。", emptyRules: "还没有自学习规则。", noResults: "没有匹配的条目。",
  loading: "正在加载词条…", refresh: "刷新", enabledTerm: "启用", disabledTerm: "已停用",
  remove: "删除", deleted: "已删除，将跳过再次学习该写法。",
  processing: "正在分析修改…", pending: "有修改等待分析", source: "原写法", term: "正确写法",
  created: "学习时间", loadFailed: "加载失败", count: (count: number) => `${count} 条`,
  listHelp: "独立于人工词典保存；删除立即生效。",
  legacyHelp: "这条旧版学习记录同时影响用户词典和易错词纠正。",
};

const english: typeof chinese = {
  title: "Self-learning",
  description: "Learn names and preferred spellings from edits made after dictation.",
  enabled: "Automatically learn corrections",
  enabledHelp: "The learning model analyzes relevant transcripts and edits alongside your existing dictionary and misrecognition rules.",
  modelRequired: "Learning starts once its model is configured.",
  unsupported: "Automatic input observation is not available on this platform. You can still manage learned corrections.",
  modelTitle: "Learning model",
  terms: "Learned entries", rules: "Learned rules", search: "Search learned entries", searchRules: "Search learned rules",
  empty: "No learned entries yet.", emptyRules: "No learned rules yet.", noResults: "No matching entries.",
  loading: "Loading corrections…", refresh: "Refresh", enabledTerm: "Enabled", disabledTerm: "Disabled",
  remove: "Delete", deleted: "Deleted. This spelling will be excluded from future learning.",
  processing: "Analyzing edits…", pending: "An edit is waiting for analysis", source: "Original spelling", term: "Correct spelling",
  created: "Learned on", loadFailed: "Could not load corrections", count: (count: number) => `${count} entries`,
  listHelp: "Stored separately from your manual dictionary. Deletions take effect immediately.",
  legacyHelp: "This legacy entry affects both the user dictionary and misrecognition rules.",
};
