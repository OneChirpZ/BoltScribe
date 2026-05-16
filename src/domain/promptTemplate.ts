import type { CorrectionRule, DictionaryEntry, PromptVariable } from "../types";

export function dictionaryFromLines(value: string): DictionaryEntry[] {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean)
    .map((term) => ({ term, aliases: [], note: "" }));
}

export function dictionaryToLines(dictionary: DictionaryEntry[]) {
  return dictionary
    .map((entry) => entry.term.trim())
    .filter(Boolean)
    .join("\n");
}

export function correctionRulesFromLines(value: string): CorrectionRule[] {
  return value
    .split(/\r?\n/)
    .map(parseCorrectionRuleLine)
    .filter((rule): rule is CorrectionRule => Boolean(rule));
}

export function correctionRulesToLines(rules: CorrectionRule[]) {
  return rules
    .filter((rule) => rule.source.trim() && rule.target.trim())
    .map((rule) => {
      const base = `"${rule.source.trim()}" -> "${rule.target.trim()}"`;
      return rule.note.trim() ? `${base} # ${rule.note.trim()}` : base;
    })
    .join("\n");
}

export function buildPromptPreview(userRequirements: string, dictionary: DictionaryEntry[], correctionRules: CorrectionRule[], variables: PromptVariable[], template: string) {
  let prompt = replaceToken(template, "{{user_requirements}}", userRequirements.trim());
  prompt = replaceToken(prompt, "{{dictionary}}", formatDictionaryPreview(dictionary));
  prompt = replaceToken(prompt, "{{correction_rules}}", formatCorrectionRulesPreview(correctionRules));
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

export function formatDictionaryPreview(dictionary: DictionaryEntry[]) {
  const rows = dictionary
    .filter((entry) => entry.term.trim())
    .map((entry) => entry.term.trim());
  return rows.length > 0 ? rows.join("\n") : "（空）";
}

export function formatCorrectionRulesPreview(rules: CorrectionRule[]) {
  const rows = rules
    .filter((rule) => rule.source.trim() && rule.target.trim())
    .map((rule) => {
      const source = rule.source.trim();
      const target = rule.target.trim();
      return rule.note.trim() ? `"${source}" -> "${target}"（${rule.note.trim()}）` : `"${source}" -> "${target}"`;
    });
  return rows.length > 0 ? rows.join("\n") : "（空）";
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

function parseCorrectionRuleLine(line: string): CorrectionRule | null {
  const trimmed = line.trim();
  if (!trimmed) {
    return null;
  }
  const noteIndex = trimmed.indexOf("#");
  const ruleText = noteIndex >= 0 ? trimmed.slice(0, noteIndex).trim() : trimmed;
  const note = noteIndex >= 0 ? trimmed.slice(noteIndex + 1).trim() : "";
  for (const marker of ["->", "=>", "→"]) {
    const index = ruleText.indexOf(marker);
    if (index < 0) {
      continue;
    }
    const source = trimRulePart(ruleText.slice(0, index));
    const target = trimRulePart(ruleText.slice(index + marker.length));
    if (source && target) {
      return { source, target, note };
    }
  }
  return null;
}

function trimRulePart(value: string) {
  return value.trim().replace(/^[`"'“”‘’]+|[`"'“”‘’]+$/g, "").trim();
}
