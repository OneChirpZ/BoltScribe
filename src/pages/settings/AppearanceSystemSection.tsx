import type { AppConfig } from "../../types";
import { applyLanguageDefaultCorrectionTemplate } from "../../domain/defaultCorrectionTemplates";
import type { AppLanguage, TextBundle } from "../../domain/i18n";
import { supportsDockVisibilityControl } from "../../domain/platform";
import { normalizeThemePreference, type ThemePreference } from "../../domain/theme";

export default function AppearanceSystemSection({
  config,
  onChange,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  function updateLanguage(value: string) {
    const language: AppLanguage = value === "en-US" ? "en-US" : "zh-CN";
    onChange(applyLanguageDefaultCorrectionTemplate(config, language));
  }

  function updateTheme(theme: ThemePreference) {
    onChange({ ...config, ui: { ...config.ui, theme } });
  }

  return (
    <div className="settings-overview-grid">
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.appearance}</h2>
        </div>
        <div className="settings-appearance-stack">
          <label className="field settings-language-field">
            <span>{text.settings.languageSection}</span>
            <select
              aria-label={text.settings.language}
              value={config.ui.app_language ?? "zh-CN"}
              onChange={(event) => updateLanguage(event.target.value)}
            >
              <option value="zh-CN">{text.settings.chinese}</option>
              <option value="en-US">{text.settings.english}</option>
            </select>
          </label>
          <fieldset className="theme-field">
            <legend>{text.settings.theme}</legend>
            <div className="theme-options">
              {([
                ["system", text.settings.themeSystem],
                ["light", text.settings.themeLight],
                ["dark", text.settings.themeDark],
              ] as const).map(([value, label]) => (
                <label className="theme-option" key={value}>
                  <input
                    type="radio"
                    name="ui-theme"
                    value={value}
                    checked={normalizeThemePreference(config.ui.theme) === value}
                    onChange={() => updateTheme(value)}
                  />
                  <span className="theme-option-content">
                    <span aria-hidden="true" className={`theme-swatch ${value}`} />
                    <span>{label}</span>
                  </span>
                </label>
              ))}
            </div>
            <small>{text.settings.themeHelp}</small>
          </fieldset>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.system}</h2>
        </div>
        <div className="settings-toggle-stack">
          <label className="toggle-row">
            <input
              type="checkbox"
              checked={config.system.launch_at_login}
              onChange={(event) => onChange({
                ...config,
                system: { ...config.system, launch_at_login: event.target.checked },
              })}
            />
            <span>{text.settings.launchAtLogin}</span>
          </label>
          {supportsDockVisibilityControl() ? (
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={config.system.hide_dock_icon ?? false}
                onChange={(event) => onChange({
                  ...config,
                  system: { ...config.system, hide_dock_icon: event.target.checked },
                })}
              />
              <span>{text.settings.hideDockIcon}</span>
            </label>
          ) : null}
        </div>
      </div>
    </div>
  );
}
