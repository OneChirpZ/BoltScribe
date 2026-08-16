import Field from "../../components/Field";
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
import type { AppConfig, AudioInputDevice } from "../../types";

export default function InputVadSection({
  config,
  audioDevices,
  onChange,
  text,
}: {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  const policy = inputDevicePolicy(config);
  const addablePriorityDevices = audioDevices.filter(
    (device) => !containsInputDeviceRef(policy.priority, device) && !containsInputDeviceRef(policy.blacklist, device),
  );
  const missingBlacklistedDevices = policy.blacklist.filter((blocked) => !findInputDevice(blocked, audioDevices));
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
    </div>
  );
}
