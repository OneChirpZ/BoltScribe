import assert from "node:assert/strict";
import test from "node:test";
import { applyThemePreference, normalizeThemePreference, resolveThemePreference } from "../src/domain/theme.ts";

test("normalizes supported theme preferences", () => {
  assert.equal(normalizeThemePreference("system"), "system");
  assert.equal(normalizeThemePreference("light"), "light");
  assert.equal(normalizeThemePreference("dark"), "dark");
});

test("falls back to the system theme for unknown values", () => {
  assert.equal(normalizeThemePreference(undefined), "system");
  assert.equal(normalizeThemePreference("sepia"), "system");
});

test("resolves system mode against the current OS appearance", () => {
  assert.equal(resolveThemePreference("system", false), "light");
  assert.equal(resolveThemePreference("system", true), "dark");
  assert.equal(resolveThemePreference("light", true), "light");
  assert.equal(resolveThemePreference("dark", false), "dark");
});

test("applies explicit themes and records the selected preference", () => {
  const root = { dataset: {} as DOMStringMap };

  applyThemePreference("light", true, root);
  assert.equal(root.dataset.theme, "light");
  assert.equal(root.dataset.themePreference, "light");

  applyThemePreference("dark", false, root);
  assert.equal(root.dataset.theme, "dark");

  applyThemePreference("system", false, root);
  assert.equal(root.dataset.theme, "light");
  assert.equal(root.dataset.themePreference, "system");
});
