import { spawnSync } from "node:child_process";
import { homedir } from "node:os";

if (process.platform !== "darwin") {
  console.error("release:macos must run on macOS");
  process.exit(1);
}

if (process.env.RUSTFLAGS && !process.env.CARGO_ENCODED_RUSTFLAGS) {
  console.error(
    "release:macos cannot safely preserve RUSTFLAGS; use CARGO_ENCODED_RUSTFLAGS instead",
  );
  process.exit(1);
}

const separator = "\x1f";
const encodedRustFlags = (process.env.CARGO_ENCODED_RUSTFLAGS ?? "")
  .split(separator)
  .filter(Boolean);
encodedRustFlags.push(`--remap-path-prefix=${homedir()}=/build-home`);

const result = spawnSync("npm", ["run", "tauri", "--", "build"], {
  env: {
    ...process.env,
    CARGO_ENCODED_RUSTFLAGS: encodedRustFlags.join(separator),
  },
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
