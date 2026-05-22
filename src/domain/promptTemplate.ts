import type { PromptVariable } from "../types";

export function buildPromptPreview(userRequirements: string, dictionaryText: string, correctionRulesText: string, variables: PromptVariable[], template: string) {
  let prompt = replaceToken(template, "{{user_requirements}}", userRequirements.trim());
  prompt = replaceToken(prompt, "{{dictionary}}", dictionaryText);
  prompt = replaceToken(prompt, "{{correction_rules}}", correctionRulesText);
  prompt = replaceToken(prompt, "{{raw_text}}", "这是一次语音输入测试。");
  for (const variable of variables ?? []) {
    const token = variableToken(variable.name);
    if (!token || isBuiltinVariable(variable.name)) {
      continue;
    }
    prompt = replaceToken(prompt, token, variable.value.trim());
  }
  return prompt;
}

export function replaceToken(template: string, token: string, value: string) {
  return template.split(token).join(value);
}

export function normalizeVariableName(value: string) {
  return value.trim().replace(/[{}]/g, "").replace(/\s+/g, "_");
}

export function variableToken(name: string) {
  const normalized = normalizeVariableName(name);
  return normalized ? `{{${normalized}}}` : "";
}

export function isBuiltinVariable(name: string) {
  return ["user_requirements", "dictionary", "correction_rules", "raw_text"].includes(normalizeVariableName(name));
}
