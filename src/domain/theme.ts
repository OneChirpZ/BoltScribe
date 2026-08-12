export type ThemePreference = "system" | "light" | "dark";

export function normalizeThemePreference(value: string | null | undefined): ThemePreference {
  return value === "light" || value === "dark" ? value : "system";
}

export function resolveThemePreference(
  value: string | null | undefined,
  prefersDark: boolean,
): Exclude<ThemePreference, "system"> {
  const preference = normalizeThemePreference(value);
  return preference === "system" ? (prefersDark ? "dark" : "light") : preference;
}

export function applyThemePreference(
  value: string | null | undefined,
  prefersDark: boolean,
  root: Pick<HTMLElement, "dataset"> = document.documentElement,
) {
  const preference = normalizeThemePreference(value);
  root.dataset.themePreference = preference;
  root.dataset.theme = resolveThemePreference(preference, prefersDark);
}
