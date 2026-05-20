import type { AppConfig } from "../types";
import { runtimePlatform, type RuntimePlatform } from "./platform";

export const hotkeySlotCount = 2;
export const shortcutModifierValues = [
  "Ctrl",
  "Alt",
  "Cmd",
  "Shift",
] as const;
export const shortcutModifiers = shortcutModifierOptions("macos");
export const keyboardShortcutKeyOptions = [
  { value: "Space", label: "Space" },
  { value: "PageUp", label: "Page Up" },
  { value: "PageDown", label: "Page Down" },
  { value: "Home", label: "Home" },
  { value: "End", label: "End" },
  { value: "Escape", label: "Esc" },
  { value: "Tab", label: "Tab" },
  { value: "Enter", label: "Enter" },
  { value: "Delete", label: "Delete" },
  { value: "Backspace", label: "Backspace" },
  { value: "ArrowUp", label: "Up" },
  { value: "ArrowDown", label: "Down" },
  { value: "ArrowLeft", label: "Left" },
  { value: "ArrowRight", label: "Right" },
  ...Array.from({ length: 12 }, (_, index) => {
    const key = `F${index + 1}`;
    return { value: key, label: key };
  }),
  ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").map((key) => ({ value: key, label: key })),
  ..."0123456789".split("").map((key) => ({ value: key, label: key })),
];
export const soundSourceShortcutKeyOptions = [
  { value: "Space", label: "Space" },
  ...Array.from({ length: 20 }, (_, index) => {
    const key = `F${index + 1}`;
    return { value: key, label: key };
  }),
  ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").map((key) => ({ value: key, label: key })),
  ..."0123456789".split("").map((key) => ({ value: key, label: key })),
];
export const mouseShortcutKeyOptions = [
  { value: "MouseMiddle", label: "Mouse Middle" },
  { value: "MouseBack", label: "Mouse Back" },
  { value: "MouseForward", label: "Mouse Forward" },
];
const allShortcutKeyOptions = [...keyboardShortcutKeyOptions, ...mouseShortcutKeyOptions];

export type ShortcutModifier = (typeof shortcutModifierValues)[number];
export type ShortcutParts = {
  modifiers: ShortcutModifier[];
  key: string;
};

export function shortcutModifierOptions(platform: RuntimePlatform = runtimePlatform()) {
  const labels: Record<ShortcutModifier, string> = platform === "windows"
    ? { Ctrl: "ctrl", Alt: "alt", Cmd: "win", Shift: "shift" }
    : platform === "macos"
      ? { Ctrl: "ctrl", Alt: "opt", Cmd: "cmd", Shift: "shift" }
      : { Ctrl: "ctrl", Alt: "alt", Cmd: "super", Shift: "shift" };
  return shortcutModifierValues.map((value) => ({ value, label: labels[value] }));
}

export function shortcutKeyOptions(platform: RuntimePlatform = runtimePlatform()) {
  return platform === "windows" ? allShortcutKeyOptions : keyboardShortcutKeyOptions;
}

export function hotkeySlots(config: AppConfig) {
  const source = config.hotkeys?.some((hotkey) => hotkey.trim()) ? config.hotkeys : [config.hotkey];
  const slots = source.slice(0, hotkeySlotCount);
  while (slots.length < hotkeySlotCount) {
    slots.push("");
  }
  return slots;
}

export function hotkeyEnabledSlots(config: AppConfig) {
  const slots = hotkeySlots(config);
  const source = config.hotkey_enabled;
  if (!source || source.length === 0) {
    return slots.map((hotkey) => Boolean(hotkey.trim()));
  }

  const enabled = source.slice(0, hotkeySlotCount);
  while (enabled.length < hotkeySlotCount) {
    enabled.push(false);
  }
  return enabled;
}

export function activeHotkeys(config: AppConfig) {
  const enabled = hotkeyEnabledSlots(config);
  return hotkeySlots(config)
    .map((hotkey) => hotkey.trim())
    .filter((hotkey, index) => enabled[index] && hotkey);
}

export function updateHotkey(config: AppConfig, index: number, value: string): AppConfig {
  const hotkeys = hotkeySlots(config);
  const hotkey_enabled = hotkeyEnabledSlots(config);
  hotkeys[index] = value;
  const primary = hotkeys.find((hotkey, i) => hotkey_enabled[i] && hotkey.trim())?.trim() ?? "";
  return {
    ...config,
    hotkey: primary,
    hotkeys,
    hotkey_enabled,
  };
}

export function updateHotkeyEnabled(config: AppConfig, index: number, enabled: boolean): AppConfig {
  const hotkeys = hotkeySlots(config);
  const hotkey_enabled = hotkeyEnabledSlots(config);
  hotkey_enabled[index] = enabled;
  const primary = hotkeys.find((hotkey, i) => hotkey_enabled[i] && hotkey.trim())?.trim() ?? "";
  return {
    ...config,
    hotkey: primary,
    hotkeys,
    hotkey_enabled,
  };
}

export function parseShortcut(value: string, platform: RuntimePlatform = runtimePlatform()): ShortcutParts {
  const modifiers = new Set<ShortcutModifier>();
  let key = "";
  for (const rawToken of value.split("+")) {
    const token = rawToken.trim();
    if (!token) {
      continue;
    }
    const modifier = normalizeModifier(token, platform);
    if (modifier) {
      modifiers.add(modifier);
    } else {
      key = normalizeShortcutKey(token);
    }
  }

  return {
    modifiers: shortcutModifierValues.filter((modifier) => modifiers.has(modifier)),
    key,
  };
}

export function formatShortcut(parts: ShortcutParts) {
  if (!parts.key) {
    return "";
  }
  return [...shortcutModifierValues.filter((modifier) => parts.modifiers.includes(modifier)), parts.key].join("+");
}

export function displayShortcut(value: string, platform?: RuntimePlatform) {
  const resolvedPlatform = platform ?? runtimePlatform();
  const parts = parseShortcut(value, resolvedPlatform);
  return displayShortcutParts(parts.modifiers, parts.key, resolvedPlatform);
}

export function displayShortcutParts(modifiers: ShortcutModifier[], key: string, platform?: RuntimePlatform) {
  const resolvedPlatform = platform ?? runtimePlatform();
  const labels = shortcutModifierOptions(resolvedPlatform)
    .filter((modifier) => modifiers.includes(modifier.value))
    .map((modifier) => modifier.label);
  if (!key) {
    return labels.length > 0 ? `${labels.join("+")}+...` : "";
  }
  const keyLabel = shortcutKeyOptions(resolvedPlatform).find((option) => option.value === key)
    ?.label ?? allShortcutKeyOptions.find((option) => option.value === key)?.label ?? key;
  return [...labels, keyLabel].join("+");
}

export function normalizeModifier(value: string, platform: RuntimePlatform = runtimePlatform()): ShortcutModifier | null {
  switch (value.toUpperCase()) {
    case "CTRL":
    case "CONTROL":
      return "Ctrl";
    case "ALT":
    case "OPTION":
      return "Alt";
    case "CMD":
    case "COMMAND":
    case "SUPER":
    case "WIN":
    case "WINDOWS":
    case "META":
      return "Cmd";
    case "COMMANDORCONTROL":
    case "COMMANDORCTRL":
    case "CMDORCONTROL":
    case "CMDORCTRL":
      return platform === "macos" ? "Cmd" : "Ctrl";
    case "SHIFT":
      return "Shift";
    default:
      return null;
  }
}

export function normalizeShortcutKey(value: string) {
  const normalized = value.trim();
  const upper = normalized.toUpperCase();
  if (/^KEY[A-Z]$/.test(upper)) {
    return upper.slice(3);
  }
  if (/^DIGIT[0-9]$/.test(upper)) {
    return upper.slice(5);
  }
  const aliases: Record<string, string> = {
    ESC: "Escape",
    RETURN: "Enter",
    UP: "ArrowUp",
    DOWN: "ArrowDown",
    LEFT: "ArrowLeft",
    RIGHT: "ArrowRight",
    MOUSEMIDDLE: "MouseMiddle",
    MIDDLEMOUSE: "MouseMiddle",
    MOUSEBUTTONMIDDLE: "MouseMiddle",
    MOUSEBACK: "MouseBack",
    BACKMOUSE: "MouseBack",
    MOUSEBUTTONBACK: "MouseBack",
    XBUTTON1: "MouseBack",
    MOUSEFORWARD: "MouseForward",
    FORWARDMOUSE: "MouseForward",
    MOUSEBUTTONFORWARD: "MouseForward",
    XBUTTON2: "MouseForward",
  };
  if (aliases[upper]) {
    return aliases[upper];
  }
  const option = allShortcutKeyOptions.find((item) => item.value.toUpperCase() === upper || item.label.toUpperCase() === upper);
  return option?.value ?? normalized;
}
