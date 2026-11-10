import type { LearningActivityState } from "../types";

export function learningActivityText(language: string) {
  return language === "en-US" ? english : chinese;
}

export function activityStateLabel(state: LearningActivityState, language: string): string {
  return (language === "en-US" ? englishStates : chineseStates)[state];
}

export function activityReasonLabel(reason: string | null, language: string): string {
  if (!reason) return "";
  return (language === "en-US" ? englishReasons : chineseReasons)[reason] ?? learningActivityText(language).otherReason;
}

const chinese = {
  title: "自学习", current: "当前任务", history: "学习历史", enabled: "自动学习纠错词",
  help: "功能说明", helpTitle: "自学习如何工作", closeHelp: "关闭",
  helpObserve: "自动学习目前在 macOS 上提供 Beta 体验，默认关闭。开启并配置模型后，BoltScribe 会短暂观察本轮语音上屏的文字及其后续修改。周围的草稿只用于在本机核对插入范围。",
  helpSend: "修改符合学习条件时，BoltScribe 将本轮上屏的文字和手动修改后的版本发送给你配置的学习模型，提取可复用的词条。",
  helpStore: "学习历史可在本机查看修改后的文字和模型结果。通过校验的词条保存在本机词典中，可以停用或删除。",
  idle: "当前没有学习任务", waiting: "等待上屏后的手动修改", running: "自学习运行中", disabled: "自学习已关闭",
  unconfigured: "待配置学习模型", unsupported: "当前平台暂不支持自动观察输入",
  configure: "管理模型", model: "学习模型", refresh: "刷新", loading: "正在加载学习记录…",
  empty: "还没有学习记录。", allRecords: "查看全部学习记录", filtered: "这条语音的学习记录",
  previous: "上一页", next: "下一页", details: "详情", elapsed: (seconds: number) => `已用 ${seconds} 秒`,
  learned: (count: number) => `学到 ${count} 个词`, page: (page: number) => `第 ${page} 页`,
  finished: "结束时间", started: "开始时间", otherReason: "本次学习未新增词条",
  observing: "正在观察上屏后的修改", queued: "等待模型分析", analyzing: "正在分析修改",
};

const english: typeof chinese = {
  title: "Self-learning", current: "Current tasks", history: "Learning history", enabled: "Automatically learn corrections",
  help: "About self-learning", helpTitle: "How self-learning works", closeHelp: "Close",
  helpObserve: "Automatic learning is in beta on macOS and is off by default. Once enabled with a model configured, BoltScribe briefly watches for edits to the text it inserted from this dictation. It uses surrounding draft text locally to verify the insertion boundaries.",
  helpSend: "When an edit qualifies for learning, BoltScribe sends the inserted text and the edited version to your configured learning model to find reusable spelling corrections.",
  helpStore: "You can review the edited text and model result in your local learning history. Accepted corrections are stored in the local dictionary, where you can disable or delete them.",
  idle: "No learning tasks in progress", waiting: "Waiting for edits after dictation", running: "Self-learning is active", disabled: "Self-learning is off",
  unconfigured: "Configure a learning model", unsupported: "Automatic observation is unavailable on this platform",
  configure: "Manage models", model: "Learning model", refresh: "Refresh", loading: "Loading learning history…",
  empty: "No learning history yet.", allRecords: "Show all learning history", filtered: "Learning for this dictation",
  previous: "Previous", next: "Next", details: "Details", elapsed: (seconds: number) => `${seconds}s elapsed`,
  learned: (count: number) => `Learned ${count} ${count === 1 ? "correction" : "corrections"}`, page: (page: number) => `Page ${page}`,
  finished: "Finished", started: "Started", otherReason: "No corrections were added",
  observing: "Observing edits after dictation", queued: "Waiting for model analysis", analyzing: "Analyzing edits",
};

const chineseStates: Record<LearningActivityState, string> = {
  observing: "观察中", queued: "等待分析", analyzing: "分析中", learned: "已学习", no_change: "未检测到修改",
  no_terms: "未新增", skipped: "已跳过", failed: "分析失败", interrupted: "已中断",
};

const englishStates: Record<LearningActivityState, string> = {
  observing: "Observing", queued: "Queued", analyzing: "Analyzing", learned: "Learned", no_change: "No edits",
  no_terms: "No corrections added", skipped: "Skipped", failed: "Failed", interrupted: "Interrupted",
};

const chineseReasons: Record<string, string> = {
  no_change: "未检测到修改", no_terms: "未提取到可学习的词条", already_known_or_conflict: "词条已存在、与已有写法冲突或已达容量上限",
  candidates_rejected: "修改未通过词条校验", invalid_sample: "修改未满足学习条件", cancelled: "已取消本次学习",
  superseded: "等待任务已被更新的修改替代", queue_expired: "等待分析超时", config_changed: "学习设置已更改",
  target_unavailable: "无法读取来源输入框", unstable_edit: "修改尚未稳定", out_of_scope: "修改超出了本轮语音范围",
  injection_mismatch: "未能从来源输入框核对上屏结果", focus_changed: "来源输入框焦点已切换",
  input_cleared: "来源输入框已清空", unrelated_edit: "修改与本轮语音差异过大",
  unsupported: "当前平台暂不支持自动观察输入", worker_unavailable: "学习服务暂不可用", app_restarted: "应用退出后，本次学习已中断",
  model_error: "模型分析失败", no_candidates: "未提取到可学习的词条", analysis_failed: "模型分析失败",
  observer_busy: "上一条语音仍在观察中", observation_interrupted: "本轮观察已中断",
  data_dir_changed: "数据目录已切换，本次学习已结束",
  source_removed: "来源语音已删除",
};

const englishReasons: Record<string, string> = {
  no_change: "No edits detected", no_terms: "No reusable corrections found", already_known_or_conflict: "Corrections already exist, conflict with saved spellings, or reached the entry limit",
  candidates_rejected: "Edits did not pass correction validation", invalid_sample: "Edits did not meet learning conditions", cancelled: "Learning was cancelled",
  superseded: "A newer edit replaced this queued task", queue_expired: "The queued task expired", config_changed: "Learning settings changed",
  target_unavailable: "The source input could not be read", unstable_edit: "Edits were still in progress", out_of_scope: "Edits extended beyond this dictation",
  injection_mismatch: "Could not verify dictation in the source field after insertion", focus_changed: "Focus moved away from the source input",
  input_cleared: "The source input was cleared", unrelated_edit: "The edit differed too much from this dictation",
  unsupported: "Automatic observation is unavailable on this platform", worker_unavailable: "The learning service is unavailable", app_restarted: "This task was interrupted when the app exited",
  model_error: "Model analysis failed", no_candidates: "No reusable corrections found", analysis_failed: "Model analysis failed",
  observer_busy: "The previous dictation is still being observed", observation_interrupted: "Observation was interrupted",
  data_dir_changed: "This learning attempt ended when the data directory changed",
  source_removed: "The source dictation was deleted",
};
