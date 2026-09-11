import { tauriEnvironment } from "./tauri-environment.mjs";

Object.assign(
  process.env,
  tauriEnvironment(process.platform, process.argv.slice(2), process.env),
);

// Run the installed CLI in this process so arguments, signals and exit status
// retain their normal behavior on macOS and Windows.
await import("@tauri-apps/cli/tauri.js");
