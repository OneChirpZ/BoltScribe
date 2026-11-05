import { spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const repoDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const installedApp = "/Applications/BoltScribe.app";
const builtApp = join(repoDir, "src-tauri/target/release/bundle/macos/BoltScribe.app");

function command(file, args, options = {}) {
  const result = spawnSync(file, args, { cwd: repoDir, encoding: "utf8", ...options });
  if (result.error || result.status !== 0) {
    throw new Error(`${basename(file)} 执行失败${result.status === null ? "" : `（${result.status}）`}`);
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
}

export function selectSigningIdentity(output, authority) {
  if (!authority?.startsWith("Apple Development:")) {
    throw new Error("已安装应用未使用稳定的 Apple Development 签名。");
  }
  const matches = [...output.matchAll(/^\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"([^"]+)"\s*$/gm)]
    .filter((match) => match[2] === authority);
  if (matches.length !== 1) {
    throw new Error("钥匙串中未找到唯一匹配的有效签名身份，请检查 Apple Development 证书。");
  }
  return matches[0][1];
}

function inspectApp(appPath) {
  command("/usr/bin/codesign", ["--verify", "--deep", "--strict", appPath]);
  const details = command("/usr/bin/codesign", ["-d", "--verbose=4", appPath]);
  const requirements = command("/usr/bin/codesign", ["-d", "-r-", appPath]);
  const authority = details.match(/^Authority=(.+)$/m)?.[1];
  const team = details.match(/^TeamIdentifier=(.+)$/m)?.[1];
  const requirement = requirements.match(/^designated => (.+)$/m)?.[1];
  if (!authority?.startsWith("Apple Development:") || !team || team === "not set"
    || !requirement?.includes("anchor apple") || /^Signature=adhoc$/m.test(details)) {
    throw new Error("应用缺少有效的 Apple Development 签名或 Apple-anchored requirement。");
  }
  const plist = (key) => command("/usr/bin/plutil", [
    "-extract", key, "raw", "-o", "-", join(appPath, "Contents/Info.plist"),
  ]);
  const executable = join(appPath, "Contents/MacOS", plist("CFBundleExecutable"));
  return {
    authority, team, requirement, executable,
    bundleId: plist("CFBundleIdentifier"),
    version: plist("CFBundleShortVersionString"),
    arch: command("/usr/bin/lipo", ["-archs", executable]).split(/\s+/).sort().join(" "),
    hash: createHash("sha256").update(readFileSync(executable)).digest("hex"),
  };
}

export function assertCompatibleApp(previous, next) {
  for (const field of ["authority", "team", "requirement", "bundleId", "arch"]) {
    if (previous[field] !== next[field]) {
      throw new Error(`新旧应用的 ${field} 不一致，已停止替换。`);
    }
  }
}

export function assertInstalledCopy(built, installed) {
  assertCompatibleApp(built, installed);
  if (built.version !== installed.version || built.hash !== installed.hash) {
    throw new Error("安装后的版本或可执行文件 SHA-256 与构建产物不一致。");
  }
}

function installedPids(executable) {
  return command("/bin/ps", ["-axo", "pid=,comm="]).split("\n")
    .map((line) => line.match(/^\s*(\d+)\s+(.+)$/))
    .filter((match) => match?.[2] === executable)
    .map((match) => Number(match[1]));
}

async function stopInstalledApp(executable) {
  const pids = installedPids(executable);
  if (pids.length === 0) return;
  // NSRunningApplication targets only the installed process and never launches an app.
  command("/usr/bin/osascript", ["-l", "JavaScript", "-e", `
    ObjC.import('AppKit');
    for (const pid of ${JSON.stringify(pids)}) {
      const app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(pid);
      if (app && !app.isTerminated && !app.terminate) throw new Error('无法退出 BoltScribe');
    }
  `], { timeout: 15_000 });
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (installedPids(executable).length === 0) return;
    await setTimeout(250);
  }
  throw new Error("BoltScribe 尚未退出；请退出应用后重新运行。旧应用未被移动。");
}

function trashPath(trashDir, label) {
  mkdirSync(trashDir, { recursive: true });
  return join(trashDir, `${label}-${new Date().toISOString().replace(/[:.]/g, "-")}-${randomUUID()}.app`);
}

export function installApp({ source, destination, trashDir, verify, copy = command }) {
  const backup = trashPath(trashDir, "BoltScribe");
  renameSync(destination, backup);
  try {
    copy("/usr/bin/ditto", [source, destination]);
    verify(destination);
  } catch (error) {
    // Keep both the old app and any failed copy recoverable; never delete bundles.
    try {
      if (existsSync(destination)) {
        renameSync(destination, trashPath(trashDir, "BoltScribe-failed"));
      }
      renameSync(backup, destination);
    } catch (recoveryError) {
      throw new Error(`替换失败且自动恢复未完成；旧应用保存在 ${backup}`, { cause: recoveryError });
    }
    throw new Error("替换失败，已恢复旧应用。", { cause: error });
  }
  return backup;
}

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("replace:macos-local 仅支持 macOS。");
  }
  if (process.argv.slice(2).some((arg) => arg !== "--check")) {
    throw new Error("用法：npm run replace:macos-local [-- --check]");
  }
  if (!existsSync(installedApp)) {
    throw new Error("未找到 /Applications/BoltScribe.app，无法确定要沿用的签名身份。");
  }
  command("npm", ["run", "version:check"], { stdio: "inherit" });
  const previous = inspectApp(installedApp);
  const config = JSON.parse(readFileSync(join(repoDir, "src-tauri/tauri.conf.json"), "utf8"));
  if (previous.bundleId !== config.identifier) {
    throw new Error("已安装应用的 Bundle ID 与项目不一致。");
  }
  const identity = selectSigningIdentity(
    command("/usr/bin/security", ["find-identity", "-v", "-p", "codesigning"]),
    previous.authority,
  );
  if (process.argv.includes("--check")) {
    console.log("本地替换检查通过：版本一致，已安装应用签名有效，钥匙串身份匹配。");
    return;
  }
  console.log("开始构建 BoltScribe 本地应用…");
  command("npm", ["run", "build:macos-local"], {
    env: { ...process.env, APPLE_SIGNING_IDENTITY: identity },
    stdio: "inherit",
  });
  const built = inspectApp(builtApp);
  assertCompatibleApp(previous, built);
  if (built.version !== config.version) throw new Error("构建产物版本与项目不一致。");
  // Recheck after the build so another installer cannot silently change our baseline.
  assertInstalledCopy(previous, inspectApp(installedApp));
  console.log("构建与签名校验通过，正在退出并替换旧应用…");
  await stopInstalledApp(previous.executable);
  const backup = installApp({
    source: builtApp, destination: installedApp, trashDir: join(homedir(), ".Trash"),
    verify: (appPath) => assertInstalledCopy(built, inspectApp(appPath)),
  });
  console.log(`BoltScribe ${built.version} 已替换，签名与 SHA-256 校验通过。应用保持关闭。`);
  console.log(`旧应用已移到废纸篓：${backup}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    if (error.cause) console.error(error.cause.message);
    process.exitCode = 1;
  });
}
