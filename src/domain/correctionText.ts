export interface TextLineRange {
  index: number;
  start: number;
  contentEnd: number;
  end: number;
  separator: "" | "\n" | "\r\n";
  raw: string;
}

export type DictionaryLine =
  | { kind: "entry"; term: string; range: TextLineRange }
  | { kind: "blank"; range: TextLineRange };

export type CorrectionRuleLine =
  | {
    kind: "rule";
    source: string;
    target: string;
    note: string;
    arrow: "->" | "=>" | "→";
    issues: Array<"source-empty" | "target-empty">;
    range: TextLineRange;
  }
  | { kind: "blank"; range: TextLineRange }
  | {
    kind: "unparsed";
    reason: "missing-arrow" | "multiple-arrows" | "unclosed-quote";
    range: TextLineRange;
  };

type ArrowMatch = { index: number; value: "->" | "=>" | "→" };

const quoteClosers: Record<string, string> = {
  "\"": "\"",
  "'": "'",
  "`": "`",
  "“": "”",
  "‘": "’",
};

export function scanTextLines(text: string): TextLineRange[] {
  const lines: TextLineRange[] = [];
  let start = 0;
  let index = 0;

  while (start < text.length) {
    const newlineIndex = text.indexOf("\n", start);
    if (newlineIndex === -1) {
      lines.push({
        index,
        start,
        contentEnd: text.length,
        end: text.length,
        separator: "",
        raw: text.slice(start),
      });
      break;
    }

    const isCrLf = newlineIndex > start && text[newlineIndex - 1] === "\r";
    const contentEnd = isCrLf ? newlineIndex - 1 : newlineIndex;
    lines.push({
      index,
      start,
      contentEnd,
      end: newlineIndex + 1,
      separator: isCrLf ? "\r\n" : "\n",
      raw: text.slice(start, contentEnd),
    });
    start = newlineIndex + 1;
    index += 1;
  }

  return lines;
}

export function parseDictionaryText(text: string): DictionaryLine[] {
  return scanTextLines(text).map((range) => {
    const term = range.raw.trim();
    return term ? { kind: "entry", term, range } : { kind: "blank", range };
  });
}

export function normalizedKey(value: string) {
  return value.trim().toLowerCase();
}

export function reconcileDisabledDictionaryTerms(text: string, disabledTerms: string[]) {
  const disabledKeys = new Set((disabledTerms ?? []).map(normalizedKey).filter(Boolean));
  const seen = new Set<string>();
  const reconciled: string[] = [];
  for (const line of parseDictionaryText(text)) {
    if (line.kind !== "entry") {
      continue;
    }
    const key = normalizedKey(line.term);
    if (!key || !disabledKeys.has(key) || seen.has(key)) {
      continue;
    }
    seen.add(key);
    reconciled.push(line.term);
  }
  return reconciled;
}

export function setDictionaryTermDisabled(text: string, disabledTerms: string[], value: string, disabled: boolean) {
  const key = normalizedKey(value);
  const disabledKeys = new Set((disabledTerms ?? []).map(normalizedKey).filter(Boolean));
  if (disabled) {
    if (key) disabledKeys.add(key);
  } else {
    disabledKeys.delete(key);
  }
  return reconcileDisabledDictionaryTerms(text, [...disabledKeys]);
}

export function enabledDictionaryText(text: string, disabledTerms: string[]) {
  const disabled = new Set((disabledTerms ?? []).map(normalizedKey).filter(Boolean));
  return parseDictionaryText(text)
    .filter((line): line is Extract<DictionaryLine, { kind: "entry" }> => line.kind === "entry")
    .map((line) => line.term)
    .filter((term) => !disabled.has(normalizedKey(term)))
    .join("\n");
}

export function parseCorrectionRulesText(text: string): CorrectionRuleLine[] {
  return scanTextLines(text).map((range) => parseCorrectionRuleLine(range));
}

export function serializeDictionaryItem(term: string) {
  return term.trim().replace(/[\r\n]+/g, " ");
}

export function serializeCorrectionRule({ source, target, note }: { source: string; target: string; note: string }) {
  const cleanSource = source.trim().replace(/[\r\n]+/g, " ");
  const cleanTarget = target.trim().replace(/[\r\n]+/g, " ");
  const cleanNote = note.trim().replace(/[\r\n]+/g, " ");
  const rule = `${JSON.stringify(cleanSource)} -> ${JSON.stringify(cleanTarget)}`;
  return cleanNote ? `${rule} # ${cleanNote}` : rule;
}

export function replaceTextLine(text: string, range: TextLineRange, nextLine: string) {
  return `${text.slice(0, range.start)}${nextLine}${text.slice(range.contentEnd)}`;
}

export function removeTextLine(text: string, range: TextLineRange) {
  if (range.separator) {
    return `${text.slice(0, range.start)}${text.slice(range.end)}`;
  }
  if (range.start === 0) {
    return "";
  }
  const prefix = text.slice(0, range.start);
  const separatorLength = prefix.endsWith("\r\n") ? 2 : prefix.endsWith("\n") ? 1 : 0;
  return `${prefix.slice(0, prefix.length - separatorLength)}${text.slice(range.end)}`;
}

export function appendTextLine(text: string, line: string) {
  if (!text) {
    return line;
  }
  return /\r?\n$/.test(text) ? `${text}${line}` : `${text}\n${line}`;
}

function parseCorrectionRuleLine(range: TextLineRange): CorrectionRuleLine {
  const raw = range.raw;
  if (!raw.trim()) {
    return { kind: "blank", range };
  }

  const syntax = scanSyntax(raw);
  if (syntax.unclosedQuote) {
    return { kind: "unparsed", reason: "unclosed-quote", range };
  }
  if (syntax.arrows.length === 0) {
    return { kind: "unparsed", reason: "missing-arrow", range };
  }
  if (syntax.arrows.length > 1) {
    return { kind: "unparsed", reason: "multiple-arrows", range };
  }

  const arrow = syntax.arrows[0];
  const sourceRaw = raw.slice(0, arrow.index);
  const targetStart = arrow.index + arrow.value.length;
  const noteHash = syntax.hashes.find((index) => index >= targetStart);
  let targetRaw = raw.slice(targetStart, noteHash ?? raw.length);
  let note = noteHash === undefined ? "" : raw.slice(noteHash + 1).trim();

  if (noteHash === undefined) {
    const legacyNote = extractLegacyNote(targetRaw);
    if (legacyNote) {
      targetRaw = legacyNote.target;
      note = legacyNote.note;
    }
  }

  const source = unwrapValue(sourceRaw);
  const target = unwrapValue(targetRaw);
  const issues: Array<"source-empty" | "target-empty"> = [];
  if (!source.trim()) {
    issues.push("source-empty");
  }
  if (!target.trim()) {
    issues.push("target-empty");
  }

  return { kind: "rule", source, target, note, arrow: arrow.value, issues, range };
}

function scanSyntax(value: string) {
  const arrows: ArrowMatch[] = [];
  const hashes: number[] = [];
  let quoteCloser = "";
  let escaped = false;

  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (quoteCloser) {
      if (escaped) {
        escaped = false;
        continue;
      }
      if (character === "\\" && quoteCloser !== "”" && quoteCloser !== "’") {
        escaped = true;
        continue;
      }
      if (character === quoteCloser) {
        quoteCloser = "";
      }
      continue;
    }

    const closer = quoteClosers[character];
    if (closer) {
      quoteCloser = closer;
      continue;
    }
    if (character === "#") {
      hashes.push(index);
      break;
    }
    if (character === "→") {
      arrows.push({ index, value: "→" });
      continue;
    }
    const pair = value.slice(index, index + 2);
    if (pair === "->" || pair === "=>") {
      arrows.push({ index, value: pair });
      index += 1;
    }
  }

  return { arrows, hashes, unclosedQuote: Boolean(quoteCloser) };
}

function extractLegacyNote(value: string) {
  const trimmed = value.trimEnd();
  if (!trimmed.endsWith("）")) {
    return null;
  }
  const openIndex = findLastOutsideQuote(trimmed, "（");
  if (openIndex < 0) {
    return null;
  }
  return {
    target: trimmed.slice(0, openIndex).trimEnd(),
    note: trimmed.slice(openIndex + 1, -1).trim(),
  };
}

function findLastOutsideQuote(value: string, needle: string) {
  let quoteCloser = "";
  let escaped = false;
  let lastIndex = -1;
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (quoteCloser) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\" && quoteCloser !== "”" && quoteCloser !== "’") {
        escaped = true;
      } else if (character === quoteCloser) {
        quoteCloser = "";
      }
      continue;
    }
    const closer = quoteClosers[character];
    if (closer) {
      quoteCloser = closer;
    } else if (character === needle) {
      lastIndex = index;
    }
  }
  return lastIndex;
}

function unwrapValue(value: string) {
  const trimmed = value.trim();
  const closer = quoteClosers[trimmed[0]];
  if (!closer || trimmed[trimmed.length - 1] !== closer) {
    return trimmed;
  }
  if (trimmed[0] === "\"") {
    try {
      return JSON.parse(trimmed) as string;
    } catch {
      return trimmed.slice(1, -1);
    }
  }
  const body = trimmed.slice(1, -1);
  return body.replace(/\\([\\'"`])/g, "$1");
}
