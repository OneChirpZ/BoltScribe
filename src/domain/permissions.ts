export type PermissionRequestState = "unknown" | "checking" | "granted" | "denied";

export function microphoneStatusLabel(status: PermissionRequestState) {
  switch (status) {
    case "checking":
      return "请求中";
    case "granted":
      return "已启用";
    case "denied":
      return "未启用";
    default:
      return "未请求";
  }
}
