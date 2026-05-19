export type RuntimePlatform = "macos" | "windows" | "linux" | "unknown";

export function runtimePlatform(): RuntimePlatform {
  const userAgentDataPlatform = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform;
  const value = `${userAgentDataPlatform ?? ""} ${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`.toLowerCase();

  if (value.includes("win")) {
    return "windows";
  }
  if (value.includes("mac")) {
    return "macos";
  }
  if (value.includes("linux")) {
    return "linux";
  }
  return "unknown";
}

export function requiresAccessibilityPermission() {
  return runtimePlatform() === "macos";
}

export function supportsDockVisibilityControl() {
  return runtimePlatform() === "macos";
}

export function supportsOutputVolumeDucking() {
  return runtimePlatform() === "macos";
}
