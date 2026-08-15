import { useEffect, useId, useRef, useState } from "react";
import HelpTip from "../../components/HelpTip";
import {
  asrConnectionRetryLimits,
  normalizeAsrConnectionRetry,
  updateAsrConnectionRetry,
  type AsrConnectionRetryKey,
} from "../../domain/asrConnectionRetry";
import type { AppConfig } from "../../types";
import type { TextBundle } from "../../domain/i18n";

export default function AdvancedSettingsSection({
  config,
  onChange,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  const [open, setOpen] = useState(false);
  const connectionRetry = normalizeAsrConnectionRetry(config.asr.connection_retry);

  function update(key: AsrConnectionRetryKey, value: string) {
    const nextConfig = updateAsrConnectionRetry(config, key, value);
    onChange(nextConfig);
    return nextConfig.asr.connection_retry[key];
  }

  return (
    <details
      className="settings-section advanced-settings-section"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className="advanced-settings-summary">
        <span>{text.settings.advancedSettings}</span>
        <span className="advanced-settings-chevron" aria-hidden="true" />
      </summary>

      {open ? (
        <div className="advanced-settings-content">
          <div className="settings-subsection-heading">
            <h3>{text.settings.asrConnectionRetry}</h3>
          </div>
          <p className="settings-help-text">{text.settings.asrConnectionRetryHelp}</p>
          <div className="advanced-settings-grid">
            <RetryNumberField
              configKey="live_post_stop_max_attempts"
              label={text.settings.asrLivePostStopMaxAttempts}
              help={text.settings.asrLivePostStopMaxAttemptsHelp}
              unit={text.settings.retryAttemptsUnit}
              value={connectionRetry.live_post_stop_max_attempts}
              onChange={update}
            />
            <RetryNumberField
              configKey="file_max_attempts"
              label={text.settings.asrFileMaxAttempts}
              help={text.settings.asrFileMaxAttemptsHelp}
              unit={text.settings.retryAttemptsUnit}
              value={connectionRetry.file_max_attempts}
              onChange={update}
            />
            <RetryNumberField
              configKey="connect_timeout_ms"
              label={text.settings.asrConnectTimeout}
              help={text.settings.asrConnectTimeoutHelp}
              unit={text.common.milliseconds}
              value={connectionRetry.connect_timeout_ms}
              onChange={update}
            />
            <RetryNumberField
              configKey="initial_delay_ms"
              label={text.settings.asrInitialRetryDelay}
              help={text.settings.asrInitialRetryDelayHelp}
              unit={text.common.milliseconds}
              value={connectionRetry.initial_delay_ms}
              onChange={update}
            />
            <RetryNumberField
              configKey="max_delay_ms"
              label={text.settings.asrMaxRetryDelay}
              help={text.settings.asrMaxRetryDelayHelp}
              unit={text.common.milliseconds}
              value={connectionRetry.max_delay_ms}
              onChange={update}
            />
          </div>
        </div>
      ) : null}
    </details>
  );
}

function RetryNumberField({
  configKey,
  label,
  help,
  unit,
  value,
  onChange,
}: {
  configKey: AsrConnectionRetryKey;
  label: string;
  help: string;
  unit: string;
  value: number;
  onChange: (key: AsrConnectionRetryKey, value: string) => number;
}) {
  const id = useId();
  const limits = asrConnectionRetryLimits[configKey];
  const [draft, setDraft] = useState(String(value));
  const editingRef = useRef(false);

  useEffect(() => {
    if (!editingRef.current) {
      setDraft(String(value));
    }
  }, [value]);

  function normalizedDraft(raw: string) {
    const parsed = Number(raw);
    if (!raw.trim() || !Number.isFinite(parsed)) {
      return value;
    }
    return Math.min(limits.max, Math.max(limits.min, Math.round(parsed)));
  }

  function commitDraft(raw: string) {
    const next = normalizedDraft(raw);
    editingRef.current = false;
    if (next !== value) {
      const applied = onChange(configKey, String(next));
      setDraft(String(applied));
      return;
    }
    setDraft(String(next));
  }

  return (
    <div className="field advanced-retry-field">
      <div className="advanced-retry-label-row">
        <label htmlFor={id}>{label}</label>
        <HelpTip content={help} />
      </div>
      <div className="number-with-unit">
        <input
          id={id}
          type="number"
          inputMode="numeric"
          min={limits.min}
          max={limits.max}
          step="1"
          value={draft}
          onFocus={() => {
            editingRef.current = true;
          }}
          onChange={(event) => {
            setDraft(event.target.value);
          }}
          onBlur={(event) => commitDraft(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.currentTarget.blur();
            }
          }}
        />
        <span>{unit}</span>
      </div>
    </div>
  );
}
