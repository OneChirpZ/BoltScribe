import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";

if (process.platform !== "darwin") {
  console.error("build:macos-local must run on macOS");
  process.exit(1);
}

const signingIdentity = process.env.APPLE_SIGNING_IDENTITY?.trim();
if (!signingIdentity || signingIdentity === "-") {
  console.error(
    "build:macos-local requires APPLE_SIGNING_IDENTITY to reference a stable Keychain identity",
  );
  process.exit(1);
}

if (process.env.RUSTFLAGS && !process.env.CARGO_ENCODED_RUSTFLAGS) {
  console.error(
    "build:macos-local cannot safely preserve RUSTFLAGS; use CARGO_ENCODED_RUSTFLAGS instead",
  );
  process.exit(1);
}

const separator = "\x1f";
const encodedRustFlags = (process.env.CARGO_ENCODED_RUSTFLAGS ?? "")
  .split(separator)
  .filter(Boolean);
encodedRustFlags.push(`--remap-path-prefix=${homedir()}=/build-home`);

const result = spawnSync(
  "npm",
  ["run", "tauri", "--", "build", "--bundles", "app"],
  {
    env: {
      ...process.env,
      CARGO_ENCODED_RUSTFLAGS: encodedRustFlags.join(separator),
    },
    stdio: "inherit",
  },
);

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const appPath = resolve(
  process.cwd(),
  "src-tauri/target/release/bundle/macos/BoltScribe.app",
);

function signatureInfo(path) {
  const details = spawnSync(
    "/usr/bin/codesign",
    ["-d", "--verbose=4", path],
    { encoding: "utf8" },
  );
  const output = `${details.stdout ?? ""}\n${details.stderr ?? ""}`;
  const rawTeam = output.match(/^TeamIdentifier=(.+)$/m)?.[1].trim() ?? null;
  const team = rawTeam === "not set" ? null : rawTeam;
  const adHoc = /^Signature=adhoc$/m.test(output) || rawTeam === "not set";
  const requirementResult = spawnSync(
    "/usr/bin/codesign",
    ["-d", "-r-", path],
    { encoding: "utf8" },
  );
  const requirementOutput = `${requirementResult.stdout ?? ""}\n${requirementResult.stderr ?? ""}`;
  const requirement = requirementOutput.match(/^designated => (.+)$/m)?.[1] ?? null;
  return {
    valid: details.status === 0 && requirementResult.status === 0,
    team,
    adHoc,
    requirement,
  };
}

const verification = spawnSync(
  "/usr/bin/codesign",
  ["--verify", "--deep", "--strict", appPath],
  { stdio: "inherit" },
);
const builtSignature = signatureInfo(appPath);
if (
  verification.status !== 0
  || !builtSignature.valid
  || builtSignature.adHoc
  || !builtSignature.team
  || !builtSignature.requirement?.includes("anchor apple")
) {
  console.error("build:macos-local produced an app without a stable Apple signing requirement");
  process.exit(1);
}

const installedApp = "/Applications/BoltScribe.app";
if (existsSync(installedApp)) {
  const installedSignature = signatureInfo(installedApp);
  if (
    installedSignature.valid
    && !installedSignature.adHoc
    && (
      installedSignature.team !== builtSignature.team
      || installedSignature.requirement !== builtSignature.requirement
    )
  ) {
    console.error("build:macos-local refused to change the installed app's signing lineage");
    process.exit(1);
  }
}

console.log("BoltScribe local app built with a stable signing requirement");
