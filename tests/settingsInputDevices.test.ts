import assert from "node:assert/strict";
import test from "node:test";
import type { AudioInputDevice } from "../src/types.ts";
import {
  addPriorityDevice,
  movePriorityDevice,
  toggleBlacklistedDevice,
  type InputDevicePolicy,
} from "../src/domain/settingsInputDevices.ts";

const microphone: AudioInputDevice = {
  id: "mic-1",
  name: "Desk Microphone",
  is_default: false,
};

test("input device policy keeps priority ordered and blacklist mutually exclusive", () => {
  const empty: InputDevicePolicy = { priority: [], blacklist: [] };
  const preferred = addPriorityDevice(empty, microphone);
  assert.deepEqual(preferred.priority, [{ id: "mic-1", name: "Desk Microphone" }]);
  assert.equal(addPriorityDevice(preferred, microphone), preferred);

  const second = addPriorityDevice(preferred, { ...microphone, id: "mic-2", name: "Room Microphone" });
  assert.deepEqual(movePriorityDevice(second, 1, -1).priority.map((item) => item.id), ["mic-2", "mic-1"]);

  const blocked = toggleBlacklistedDevice(second, { id: "mic-1", name: "Desk Microphone" }, true);
  assert.deepEqual(blocked.priority.map((item) => item.id), ["mic-2"]);
  assert.deepEqual(blocked.blacklist.map((item) => item.id), ["mic-1"]);
});
