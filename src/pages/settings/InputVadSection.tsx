import AudioWaveform from "../../components/AudioWaveform";
import Field from "../../components/Field";
import HelpTip from "../../components/HelpTip";
import type { TextBundle } from "../../domain/i18n";
import {
  addPriorityDevice,
  applyInputDevicePolicy,
  containsInputDeviceRef,
  findInputDevice,
  inputDevicePolicy,
  inputDeviceRef,
  movePriorityDevice,
  removePriorityDevice,
  toggleBlacklistedDevice,
  type InputDevicePolicy,
} from "../../domain/settingsInputDevices";
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
import type { AppConfig, AudioInputDevice, VadTestStatus, VoiceActivityDetectionConfig } from "../../types";
import { clampUnitInterval, formatVadDbfs, VadFieldLabel, VadRangeNumberControl } from "./VadControls";
import { useVadTest } from "./useVadTest";

export default function InputVadSection({
  config,
  audioDevices,
  onChange,
  vadTestStatus,
  onStartVadTest,
  onUpdateVadTestSettings,
  onStopVadTest,
  text,
}: {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  onChange: (config: AppConfig) => void;
  vadTestStatus: VadTestStatus;
  onStartVadTest: () => void;
  onUpdateVadTestSettings: (noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number) => void;
  onStopVadTest: () => void;
  text: TextBundle;
}) {
  const policy = inputDevicePolicy(config);
  const addablePriorityDevices = audioDevices.filter(
    (device) => !containsInputDeviceRef(policy.priority, device) && !containsInputDeviceRef(policy.blacklist, device),
  );
  const missingBlacklistedDevices = policy.blacklist.filter((blocked) => !findInputDevice(blocked, audioDevices));
  const savedVad = config.audio.voice_activity_detection;
  const vad: VoiceActivityDetectionConfig = {
    enabled: savedVad?.enabled ?? defaultVadEnabled,
    noise_margin_db: savedVad?.noise_margin_db ?? defaultVadNoiseMarginDb,
    confirmation_ms: savedVad?.confirmation_ms ?? defaultVadConfirmationMs,
    noise_window_ms: savedVad?.noise_window_ms ?? defaultVadNoiseWindowMs,
    initial_silence_timeout_secs: savedVad?.initial_silence_timeout_secs ?? defaultVadInitialSilenceTimeoutSecs,
  };
  const vadTest = useVadTest(vadTestStatus, onStopVadTest, text);

  function updatePolicy(next: InputDevicePolicy) {
    onChange(applyInputDevicePolicy(config, next));
  }

  function addPriority(id: string) {
    const device = audioDevices.find((item) => item.id === id);
    if (device) {
      const next = addPriorityDevice(policy, device);
      if (next !== policy) {
        updatePolicy(next);
      }
    }
  }

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
    <div className="settings-subsection audio-input-subsection">
      <div className="settings-subsection-heading">
        <h3>{text.settings.audioInput}</h3>
      </div>
      <div className="audio-input-policy">
        <Field label={text.settings.audioInputPriority} className="audio-policy-card" group>
          <div className="input-device-priority-list">
            {policy.priority.map((preferred, index) => {
              const available = findInputDevice(preferred, audioDevices);
              return (
                <div className="input-device-priority-row" key={`${preferred.id}:${preferred.name}`}>
                  <span className="input-device-rank">{index + 1}</span>
                  <span className="input-device-name">
                    {available?.name ?? (preferred.name || preferred.id)}
                    {available?.is_default ? text.settings.audioDefaultBadge : ""}
                    {!available ? text.settings.audioPolicyMissingBadge : ""}
                  </span>
                  <div className="input-device-row-actions">
                    <button
                      className="secondary small input-device-order-button"
                      type="button"
                      disabled={index === 0}
                      title={text.settings.audioPriorityMoveUp}
                      aria-label={text.settings.audioPriorityMoveUp}
                      onClick={() => updatePolicy(movePriorityDevice(policy, index, -1))}
                    >
                      ↑
                    </button>
                    <button
                      className="secondary small input-device-order-button"
                      type="button"
                      disabled={index === policy.priority.length - 1}
                      title={text.settings.audioPriorityMoveDown}
                      aria-label={text.settings.audioPriorityMoveDown}
                      onClick={() => updatePolicy(movePriorityDevice(policy, index, 1))}
                    >
                      ↓
                    </button>
                    <button
                      className="secondary small"
                      type="button"
                      onClick={() => updatePolicy(removePriorityDevice(policy, index))}
                    >
                      {text.settings.audioPriorityRemove}
                    </button>
                  </div>
                </div>
              );
            })}
            {policy.priority.length === 0 ? <span>{text.settings.audioPriorityEmpty}</span> : null}
            <select
              aria-label={text.settings.audioPriorityAdd}
              value=""
              disabled={addablePriorityDevices.length === 0}
              onChange={(event) => addPriority(event.target.value)}
            >
              <option value="">{text.settings.audioPriorityAdd}</option>
              {addablePriorityDevices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}
                </option>
              ))}
            </select>
          </div>
        </Field>

        <Field label={text.settings.audioInputBlacklist} className="audio-policy-card" group>
          <div className="device-checklist audio-device-checklist">
            {audioDevices.map((device) => (
              <label key={device.id} className="device-checklist-row">
                <input
                  type="checkbox"
                  checked={containsInputDeviceRef(policy.blacklist, device)}
                  onChange={(event) => updatePolicy(toggleBlacklistedDevice(policy, inputDeviceRef(device), event.target.checked))}
                />
                <span>{device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}</span>
              </label>
            ))}
            {missingBlacklistedDevices.map((blocked) => (
              <label key={`${blocked.id}:${blocked.name}`} className="device-checklist-row">
                <input
                  type="checkbox"
                  checked
                  onChange={(event) => updatePolicy(toggleBlacklistedDevice(policy, blocked, event.target.checked))}
                />
                <span>{blocked.name || blocked.id}{text.settings.audioPolicyMissingBadge}</span>
              </label>
            ))}
            {audioDevices.length === 0 && missingBlacklistedDevices.length === 0
              ? <span>{text.settings.audioNoInputDevices}</span>
              : null}
          </div>
        </Field>
        <p className="settings-help-text">{text.settings.audioInputPolicyHelp}</p>
      </div>

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
    </div>
  );
}
