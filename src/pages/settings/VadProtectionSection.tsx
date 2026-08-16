import AudioWaveform from "../../components/AudioWaveform";
import Field from "../../components/Field";
import HelpTip from "../../components/HelpTip";
import type { TextBundle } from "../../domain/i18n";
import {
  defaultVadConfirmationMs,
  defaultVadEnabled,
  defaultVadInitialSilenceTimeoutSecs,
  defaultVadNoiseMarginDb,
  defaultVadNoiseWindowMs,
  maxVadConfirmationMs,
  maxVadInitialSilenceTimeoutSecs,
  maxVadNoiseMarginDb,
  maxVadNoiseWindowMs,
  minVadConfirmationMs,
  minVadInitialSilenceTimeoutSecs,
  minVadNoiseMarginDb,
  minVadNoiseWindowMs,
  vadConfirmationStepMs,
  vadInitialSilenceTimeoutStepSecs,
  vadNoiseMarginStepDb,
  vadNoiseWindowStepMs,
} from "../../domain/vadSettings";
import type { AppConfig, VadTestStatus, VoiceActivityDetectionConfig } from "../../types";
import { clampUnitInterval, formatVadDbfs, VadFieldLabel, VadRangeNumberControl } from "./VadControls";
import { useVadTest } from "./useVadTest";

export default function VadProtectionSection({
  config,
  onChange,
  vadTestStatus,
  onStartVadTest,
  onUpdateVadTestSettings,
  onStopVadTest,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  vadTestStatus: VadTestStatus;
  onStartVadTest: () => void;
  onUpdateVadTestSettings: (noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number) => void;
  onStopVadTest: () => void;
  text: TextBundle;
}) {
  const savedVad = config.audio.voice_activity_detection;
  const vad: VoiceActivityDetectionConfig = {
    enabled: savedVad?.enabled ?? defaultVadEnabled,
    noise_margin_db: savedVad?.noise_margin_db ?? defaultVadNoiseMarginDb,
    confirmation_ms: savedVad?.confirmation_ms ?? defaultVadConfirmationMs,
    noise_window_ms: savedVad?.noise_window_ms ?? defaultVadNoiseWindowMs,
    initial_silence_timeout_secs: savedVad?.initial_silence_timeout_secs ?? defaultVadInitialSilenceTimeoutSecs,
  };
  const vadTest = useVadTest(vadTestStatus, onStopVadTest, text);

  function updateVad(patch: Partial<VoiceActivityDetectionConfig>) {
    onChange({
      ...config,
      audio: {
        ...config.audio,
        voice_activity_detection: { ...vad, ...patch },
      },
    });
  }

  function updateVadGate(
    patch: Partial<Pick<VoiceActivityDetectionConfig, "noise_margin_db" | "confirmation_ms" | "noise_window_ms">>,
  ) {
    const next = { ...vad, ...patch };
    updateVad(patch);
    if (vadTest.running) {
      onUpdateVadTestSettings(next.noise_margin_db, next.confirmation_ms, next.noise_window_ms);
    }
  }

  return (
    <div className="vad-protection-card">
      <div className="vad-card-header">
        <div className="vad-card-copy settings-subsection-heading">
          <h3>{text.settings.vadTitle}</h3>
          <span className="beta-badge">{text.settings.vadBetaBadge}</span>
          <HelpTip content={text.settings.vadHelp} />
        </div>
        <label className="toggle-row vad-enabled-toggle">
          <input
            type="checkbox"
            checked={vad.enabled}
            onChange={(event) => updateVad({ enabled: event.target.checked })}
            disabled={vadTest.running}
          />
          <span>{text.settings.vadEnabled}</span>
        </label>
      </div>
      {vad.enabled ? (
        <div className="vad-controls-grid">
          <Field
            label={<VadFieldLabel label={text.settings.vadInitialSilenceTimeout} help={text.settings.vadInitialSilenceTimeoutHelp} />}
            className="vad-range-field"
            group
          >
            <VadRangeNumberControl
              label={text.settings.vadInitialSilenceTimeout}
              value={vad.initial_silence_timeout_secs}
              min={minVadInitialSilenceTimeoutSecs}
              max={maxVadInitialSilenceTimeoutSecs}
              step={vadInitialSilenceTimeoutStepSecs}
              unit={text.common.seconds}
              onChange={(value) => updateVad({ initial_silence_timeout_secs: value })}
            />
          </Field>
          <Field
            label={<VadFieldLabel label={text.settings.vadNoiseMargin} help={text.settings.vadNoiseMarginHelp} />}
            className="vad-range-field"
            group
          >
            <VadRangeNumberControl
              label={text.settings.vadNoiseMargin}
              value={vad.noise_margin_db}
              min={minVadNoiseMarginDb}
              max={maxVadNoiseMarginDb}
              step={vadNoiseMarginStepDb}
              unit="dB"
              onChange={(value) => updateVadGate({ noise_margin_db: value })}
            />
          </Field>
          <Field
            label={<VadFieldLabel label={text.settings.vadConfirmationDuration} help={text.settings.vadConfirmationDurationHelp} />}
            className="vad-range-field"
            group
          >
            <VadRangeNumberControl
              label={text.settings.vadConfirmationDuration}
              value={vad.confirmation_ms}
              min={minVadConfirmationMs}
              max={maxVadConfirmationMs}
              step={vadConfirmationStepMs}
              unit={text.common.milliseconds}
              onChange={(value) => updateVadGate({ confirmation_ms: value })}
            />
          </Field>
          <Field
            label={<VadFieldLabel label={text.settings.vadNoiseWindow} help={text.settings.vadNoiseWindowHelp} />}
            className="vad-range-field"
            group
          >
            <VadRangeNumberControl
              label={text.settings.vadNoiseWindow}
              value={vad.noise_window_ms}
              min={minVadNoiseWindowMs}
              max={maxVadNoiseWindowMs}
              step={vadNoiseWindowStepMs}
              unit={text.common.milliseconds}
              onChange={(value) => updateVadGate({ noise_window_ms: value })}
            />
          </Field>
        </div>
      ) : null}
      <div className="vad-test-footer">
        <div className={`vad-test-capsule ${vadTest.visualState}`}>
          <span className="vad-test-label" role="status" aria-live="polite">{vadTest.label}</span>
          <i aria-hidden="true" />
          <div className="voice-visual" aria-label={text.settings.vadTesting} role="img">
            <AudioWaveform samples={vadTest.levelHistory} />
          </div>
        </div>
        <div className="vad-test-actions">
          {!vadTest.running ? (
            <button className="secondary small" type="button" onClick={onStartVadTest}>{text.settings.vadTest}</button>
          ) : (
            <button className="secondary small" type="button" onClick={onStopVadTest}>{text.settings.vadStopTest}</button>
          )}
          <HelpTip content={text.settings.vadTestHelp} />
        </div>
      </div>
      {vadTest.running ? (
        <div className="vad-test-diagnostics" aria-label={text.settings.vadTesting}>
          <span>{text.settings.vadCurrentLevel(formatVadDbfs(vadTestStatus.level))}</span>
          <span>{text.settings.vadNoiseFloor(formatVadDbfs(vadTestStatus.noise_floor))}</span>
          <span>{text.settings.vadTriggerThreshold(formatVadDbfs(vadTestStatus.trigger_threshold))}</span>
          <span>{text.settings.vadTriggerProgress(Math.round(clampUnitInterval(vadTestStatus.trigger_progress) * 100))}</span>
        </div>
      ) : null}
    </div>
  );
}
