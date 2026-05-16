import type { WorkflowStatus } from "../types";

export const emptyStatus: WorkflowStatus = {
  mode: "idle",
  message: "就绪",
  current_audio_path: null,
  last_record_id: null,
};
