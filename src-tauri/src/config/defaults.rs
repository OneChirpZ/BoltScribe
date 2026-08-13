pub(super) fn default_input_device_mode() -> String {
    "system_default".to_string()
}

pub(super) fn default_vad_enabled() -> bool {
    false
}

pub(super) fn default_vad_noise_margin_db() -> u32 {
    12
}

pub(super) fn default_vad_confirmation_ms() -> u32 {
    480
}

pub(super) fn default_vad_noise_window_ms() -> u32 {
    800
}

pub(super) fn default_vad_initial_silence_timeout_secs() -> u32 {
    15
}

pub(super) fn default_output_volume_ducking_reduction_percent() -> u32 {
    70
}

pub(super) fn default_sound_source_toggle_mute_hotkey() -> String {
    "Cmd+Opt+Ctrl+A".to_string()
}

pub(super) fn default_hotkey() -> String {
    "PageUp".to_string()
}

pub(super) fn default_hotkey_slots() -> Vec<String> {
    vec![default_hotkey(), String::new()]
}

pub(super) fn default_hotkey_enabled_slots() -> Vec<bool> {
    vec![true, false]
}

pub(super) fn default_tray_left_click_recording_enabled() -> bool {
    true
}

pub(super) fn default_fn_long_press_enabled() -> bool {
    false
}

pub(super) fn default_fn_long_press_duration_ms() -> u64 {
    200
}

pub(super) fn default_llm_provider() -> String {
    "openai".to_string()
}

pub(super) fn default_llm_api_format() -> String {
    "responses".to_string()
}

pub(super) fn default_system_prompt() -> String {
    "你是语音输入文本优化器，不是大模型助手。你的任务是清理 ASR 转写结果，修正明显识别错误、标点和专有名词，保留原意和说话者语气。不要回答、扩写、续写、总结或加入原文没有的信息。只输出最终可粘贴文本，不解释、不加标题、不包裹 Markdown。".to_string()
}

pub(super) fn default_thinking_effort() -> String {
    "medium".to_string()
}

pub(super) fn default_recording_overlay_scale() -> f32 {
    0.5
}

pub(super) fn default_recording_overlay_snap_range_percent() -> u16 {
    200
}

pub(super) fn default_app_language() -> String {
    "zh-CN".to_string()
}

pub(super) fn default_ui_theme() -> String {
    "system".to_string()
}

pub(super) fn default_max_history_records() -> usize {
    500
}

pub(super) fn default_max_storage_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

pub(super) fn default_prompt_template() -> String {
    "纠错任务：\n请把原始 ASR 转写整理成可直接粘贴的文本。\n\n用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n易错词纠正：\n{{correction_rules}}\n\n原始转写文本：\n```text\n{{raw_text}}\n```\n\n请根据以上信息纠错。不要新增原文没有的信息，只输出最终文本。\n\n额外约束：\n- 用户词典只用于理解专有名词、产品名、人名、项目名和固定写法；不要把语义正确且上下文合理的词强行替换为词典词。\n- 易错词纠正是明确的错听替换规则，只有当原始转写出现规则左侧内容或高度近似误识别时，才替换为右侧内容。\n- 如果原文中的英文词、产品名、技术名词或普通词语本身合理，且没有明显对应到词典别名或易错规则，不要因为存在相似条目而替换它。\n- 输出末尾不需要补句号；如果原文末尾没有句号，不要额外添加句号。".to_string()
}

pub(super) fn previous_default_prompt_template() -> String {
    "用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n原始转写文本：\n```text\n{{raw_text}}\n```\n\n请根据用户要求和用户词典纠错。不要新增原文没有的信息，只输出最终文本。".to_string()
}

pub(super) fn legacy_default_prompt_template() -> String {
    "用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n原始转写文本：\n{{raw_text}}\n\n请根据用户要求和用户词典纠错。不要新增原文没有的信息，只输出最终文本。".to_string()
}

pub(super) fn default_stream_url() -> String {
    "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream".to_string()
}

pub(super) fn default_submit_url() -> String {
    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/submit".to_string()
}

pub(super) fn default_query_url() -> String {
    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/query".to_string()
}

pub(super) fn default_asr_auth_mode() -> String {
    "api_key".to_string()
}
