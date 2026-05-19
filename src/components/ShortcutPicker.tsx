import { useEffect, useState } from "react";
import { displayShortcutParts, formatShortcut, parseShortcut, shortcutKeyOptions, shortcutModifierOptions } from "../domain/hotkeys";
import type { ShortcutModifier } from "../domain/hotkeys";
import type { TextBundle } from "../domain/i18n";
import { runtimePlatform } from "../domain/platform";

export default function ShortcutPicker({
  label,
  enabled,
  value,
  onEnabledChange,
  onChange,
  text,
}: {
  label: string;
  enabled: boolean;
  value: string;
  onEnabledChange: (enabled: boolean) => void;
  onChange: (value: string) => void;
  text: TextBundle;
}) {
  const platform = runtimePlatform();
  const parts = parseShortcut(value, platform);
  const modifierOptions = shortcutModifierOptions(platform);
  const platformKeyOptions = shortcutKeyOptions(platform);
  const [draftModifiers, setDraftModifiers] = useState<ShortcutModifier[]>(parts.modifiers);
  useEffect(() => {
    setDraftModifiers(parts.modifiers);
  }, [value]);
  const options = platformKeyOptions.some((option) => option.value === parts.key) || !parts.key
    ? platformKeyOptions
    : [{ value: parts.key, label: parts.key }, ...platformKeyOptions];
  const activeModifiers = parts.key ? parts.modifiers : draftModifiers;

  function setKey(key: string) {
    onChange(formatShortcut({ modifiers: activeModifiers, key }));
  }

  function toggleModifier(modifier: ShortcutModifier) {
    const modifiers = activeModifiers.includes(modifier)
      ? activeModifiers.filter((item) => item !== modifier)
      : [...activeModifiers, modifier];
    if (parts.key) {
      onChange(formatShortcut({ ...parts, modifiers }));
    } else {
      setDraftModifiers(modifiers);
    }
  }

  return (
    <div className={enabled ? "shortcut-picker" : "shortcut-picker disabled"}>
      <div className="shortcut-picker-header">
        <span className="shortcut-picker-title">{label}</span>
        <label className="shortcut-enable">
          <input type="checkbox" checked={enabled} onChange={(event) => onEnabledChange(event.target.checked)} />
          {text.common.enabled}
        </label>
        <code>{enabled ? displayShortcutParts(activeModifiers, parts.key, platform) || text.common.unset : text.common.disabled}</code>
      </div>
      <div className="shortcut-controls">
        <div className="shortcut-modifiers">
          {modifierOptions.map((modifier) => (
            <button
              key={modifier.value}
              type="button"
              className={activeModifiers.includes(modifier.value) ? "modifier-button active" : "modifier-button"}
              aria-pressed={activeModifiers.includes(modifier.value)}
              disabled={!enabled}
              onClick={() => toggleModifier(modifier.value)}
            >
              {modifier.label}
            </button>
          ))}
        </div>
        <select className="shortcut-key-select" value={parts.key} disabled={!enabled} onChange={(event) => setKey(event.target.value)}>
          <option value="">{text.common.unset}</option>
          {options.map((option) => (
            <option key={option.value} value={option.value}>{option.label}</option>
          ))}
        </select>
      </div>
    </div>
  );
}
