// Tauri's DMG bundler checks the environment, not just the CLI --ci flag.
// Force the non-GUI path even when a parent shell enables Finder cosmetics.
export function tauriEnvironment(platform, args, environment) {
  const result = { ...environment };
  const command = args.find((arg) => !arg.startsWith("-"));
  if (platform === "darwin" && ["build", "bundle"].includes(command)) {
    result.CI = "true";
    result.TAURI_BUNDLER_DMG_IGNORE_CI = "false";
  }
  return result;
}
