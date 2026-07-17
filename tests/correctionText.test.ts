import assert from "node:assert/strict";
import test from "node:test";
import {
  appendTextLine,
  enabledDictionaryText,
  parseCorrectionRulesText,
  parseDictionaryText,
  reconcileDisabledDictionaryTerms,
  removeTextLine,
  replaceTextLine,
  scanTextLines,
  serializeCorrectionRule,
  setDictionaryTermDisabled,
} from "../src/domain/correctionText.ts";

test("scans CRLF lines and preserves untouched text during replacement", () => {
  const text = "BoltScribe\r\n\r\nCodex";
  const lines = scanTextLines(text);
  assert.equal(lines.length, 3);
  assert.equal(lines[0].separator, "\r\n");
  assert.equal(replaceTextLine(text, lines[2], "LDFC"), "BoltScribe\r\n\r\nLDFC");
});

test("dictionary parsing treats each non-empty physical line as an item", () => {
  const lines = parseDictionaryText(" BoltScribe \n\n#literal");
  assert.deepEqual(lines.map((line) => line.kind), ["entry", "blank", "entry"]);
  assert.equal(lines[0].kind === "entry" ? lines[0].term : "", "BoltScribe");
});

test("disabled dictionary terms are trimmed, case-insensitively deduplicated, and removed when no matching term remains", () => {
  assert.deepEqual(
    reconcileDisabledDictionaryTerms(" BoltScribe \r\nCodex\ncodex\n", [" Codex ", "Codex", "Missing", "codex"]),
    ["Codex"],
  );
  assert.deepEqual(
    reconcileDisabledDictionaryTerms("BoltScribe\nLDFC", ["Codex", "LDFC"]),
    ["LDFC"],
  );
});

test("disabling and enabling a dictionary term does not delete it", () => {
  const dictionaryText = "Codex\ncodex\nLDFC\nCodex CLI";
  assert.deepEqual(setDictionaryTermDisabled(dictionaryText, [], "codex", true), ["Codex"]);
  const disabled = setDictionaryTermDisabled(dictionaryText, ["codex"], " LDFC ", true);
  assert.deepEqual(disabled, ["Codex", "LDFC"]);
  assert.deepEqual(setDictionaryTermDisabled(dictionaryText, disabled, "Codex", false), ["LDFC"]);
});

test("prompt dictionary text excludes case-insensitive exact matches but not longer terms", () => {
  assert.equal(
    enabledDictionaryText(" BoltScribe \r\n\r\nCodex\ncodex\nCodex CLI ", [" Codex "]),
    "BoltScribe\nCodex CLI",
  );
});

test("parses supported arrows, quotes, current notes, and legacy notes", () => {
  const parsed = parseCorrectionRulesText([
    "\"艾迪\" -> \"ID\" # 英文缩写",
    "'扣得死' => `Codex`",
    "‘包次’ → “BoltScribe” （产品名）",
  ].join("\n"));
  const rules = parsed.filter((line) => line.kind === "rule");
  assert.deepEqual(rules.map((line) => [line.source, line.target, line.note]), [
    ["艾迪", "ID", "英文缩写"],
    ["扣得死", "Codex", ""],
    ["包次", "BoltScribe", "产品名"],
  ]);
});

test("ignores arrows and hashes inside quotes", () => {
  const [line] = parseCorrectionRulesText("\"a->b\" -> \"c#d\" # note may mention x -> y");
  assert.equal(line.kind, "rule");
  if (line.kind === "rule") {
    assert.equal(line.source, "a->b");
    assert.equal(line.target, "c#d");
    assert.equal(line.note, "note may mention x -> y");
  }
});

test("keeps ambiguous and malformed rules as unparsed lines", () => {
  const parsed = parseCorrectionRulesText("说明文字\na -> b -> c\n\"a -> b");
  assert.deepEqual(parsed.map((line) => line.kind === "unparsed" ? line.reason : line.kind), [
    "missing-arrow",
    "multiple-arrows",
    "unclosed-quote",
  ]);
});

test("serializes escaped values into a parseable canonical rule", () => {
  const serialized = serializeCorrectionRule({ source: "a\\\"b", target: "中→文", note: "备注" });
  const [line] = parseCorrectionRulesText(serialized);
  assert.equal(line.kind, "rule");
  if (line.kind === "rule") {
    assert.equal(line.source, "a\\\"b");
    assert.equal(line.target, "中→文");
    assert.equal(line.note, "备注");
  }
});

test("append and remove only touch the selected physical line", () => {
  assert.equal(appendTextLine("", "BoltScribe"), "BoltScribe");
  assert.equal(appendTextLine("BoltScribe\n", "Codex"), "BoltScribe\nCodex");
  const text = "one\nkeep blank\nlast";
  const lines = scanTextLines(text);
  assert.equal(removeTextLine(text, lines[1]), "one\nlast");
  assert.equal(removeTextLine(text, lines[2]), "one\nkeep blank");
});

test("editing a parsed rule preserves surrounding unparsed and blank lines exactly", () => {
  const text = "manual note\r\n\r\n\"old\" => \"new\" # keep\r\ntrailing prose";
  const parsed = parseCorrectionRulesText(text);
  const rule = parsed.find((line) => line.kind === "rule");
  assert.ok(rule && rule.kind === "rule");
  const updated = replaceTextLine(text, rule.range, serializeCorrectionRule({ source: "before", target: "after", note: "edited" }));
  assert.equal(updated, "manual note\r\n\r\n\"before\" -> \"after\" # edited\r\ntrailing prose");
});
