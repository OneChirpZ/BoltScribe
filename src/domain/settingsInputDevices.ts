import type { AppConfig, AudioInputDevice, AudioInputDeviceRef } from "../types";

export interface InputDevicePolicy {
  priority: AudioInputDeviceRef[];
  blacklist: AudioInputDeviceRef[];
}

export function inputDevicePolicy(config: AppConfig): InputDevicePolicy {
  return {
    priority: config.audio.input_device_priority ?? [],
    blacklist: config.audio.input_device_blacklist ?? [],
  };
}

export function applyInputDevicePolicy(config: AppConfig, policy: InputDevicePolicy): AppConfig {
  const first = policy.priority[0] ?? null;
  return {
    ...config,
    audio: {
      ...config.audio,
      input_device_mode: first ? "manual" : "system_default",
      input_device_id: first?.id || null,
      input_device_name: first?.name || null,
      input_device_priority: policy.priority,
      input_device_blacklist: policy.blacklist,
    },
  };
}

export function addPriorityDevice(policy: InputDevicePolicy, device: AudioInputDevice): InputDevicePolicy {
  if (containsInputDeviceRef(policy.priority, device) || containsInputDeviceRef(policy.blacklist, device)) {
    return policy;
  }
  return { ...policy, priority: [...policy.priority, inputDeviceRef(device)] };
}

export function movePriorityDevice(policy: InputDevicePolicy, index: number, direction: -1 | 1): InputDevicePolicy {
  const target = index + direction;
  if (index < 0 || index >= policy.priority.length || target < 0 || target >= policy.priority.length) {
    return policy;
  }
  const priority = [...policy.priority];
  [priority[index], priority[target]] = [priority[target], priority[index]];
  return { ...policy, priority };
}

export function removePriorityDevice(policy: InputDevicePolicy, index: number): InputDevicePolicy {
  if (index < 0 || index >= policy.priority.length) {
    return policy;
  }
  return { ...policy, priority: policy.priority.filter((_, itemIndex) => itemIndex !== index) };
}

export function toggleBlacklistedDevice(
  policy: InputDevicePolicy,
  device: AudioInputDeviceRef,
  checked: boolean,
): InputDevicePolicy {
  const blacklist = checked
    ? uniqueInputDeviceRefs([...policy.blacklist, device])
    : policy.blacklist.filter((blocked) => !inputDeviceRefsMatch(blocked, device));
  const priority = checked
    ? policy.priority.filter((preferred) => !inputDeviceRefsMatch(preferred, device))
    : policy.priority;
  return { priority, blacklist };
}

export function inputDeviceRef(device: AudioInputDevice): AudioInputDeviceRef {
  return { id: device.id, name: device.name };
}

export function inputDeviceRefsMatch(left: AudioInputDeviceRef, right: AudioInputDeviceRef): boolean {
  return (Boolean(left.id) && left.id === right.id)
    || (Boolean(left.name) && Boolean(right.name) && left.name.toLocaleLowerCase() === right.name.toLocaleLowerCase());
}

export function containsInputDeviceRef(
  items: AudioInputDeviceRef[],
  device: AudioInputDevice | AudioInputDeviceRef,
): boolean {
  return items.some((item) => inputDeviceRefsMatch(item, device));
}

export function findInputDevice(reference: AudioInputDeviceRef, devices: AudioInputDevice[]): AudioInputDevice | null {
  if (reference.id) {
    const exact = devices.find((device) => device.id === reference.id);
    if (exact) {
      return exact;
    }
  }
  return reference.name
    ? devices.find((device) => device.name.toLocaleLowerCase() === reference.name.toLocaleLowerCase()) ?? null
    : null;
}

function uniqueInputDeviceRefs(items: AudioInputDeviceRef[]): AudioInputDeviceRef[] {
  const unique: AudioInputDeviceRef[] = [];
  for (const item of items) {
    if (!containsInputDeviceRef(unique, item)) {
      unique.push(item);
    }
  }
  return unique;
}
