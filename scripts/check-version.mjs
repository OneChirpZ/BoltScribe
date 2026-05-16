import { readFile } from "node:fs/promises";

const semverPattern = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

const packageJson = JSON.parse(await readFile("package.json", "utf8"));
const tauriConfig = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
const cargoToml = await readFile("src-tauri/Cargo.toml", "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const versions = [
  ["package.json", packageJson.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ["src-tauri/Cargo.toml", cargoVersion],
];

const invalid = versions.filter(([, version]) => !version || !semverPattern.test(version));
if (invalid.length > 0) {
  for (const [file, version] of invalid) {
    console.error(`${file} has invalid semver version: ${version ?? "(missing)"}`);
  }
  process.exit(1);
}

const uniqueVersions = new Set(versions.map(([, version]) => version));
if (uniqueVersions.size !== 1) {
  console.error("Project versions are out of sync:");
  for (const [file, version] of versions) {
    console.error(`- ${file}: ${version}`);
  }
  process.exit(1);
}

console.log(`BoltScribe version ${packageJson.version}`);
