import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tauriEnvironment } from "./tauri-environment.mjs";

test("macOS build and standalone bundle always skip Finder cosmetics", () => {
  for (const command of ["build", "bundle"]) {
    for (const inherited of [{}, { CI: "false", TAURI_BUNDLER_DMG_IGNORE_CI: "true" }]) {
      const result = tauriEnvironment("darwin", [command, "--bundles", "dmg"], inherited);
      assert.equal(result.CI, "true");
      assert.equal(result.TAURI_BUNDLER_DMG_IGNORE_CI, "false");
    }
  }
});

test("global verbosity flags do not bypass quiet packaging", () => {
  const result = tauriEnvironment("darwin", ["-vv", "build"], {});
  assert.equal(result.CI, "true");
  assert.equal(result.TAURI_BUNDLER_DMG_IGNORE_CI, "false");
});

test("quiet packaging preserves signing and privacy-remapping settings without mutating input", () => {
  const inherited = Object.freeze({
    CI: "false",
    TAURI_BUNDLER_DMG_IGNORE_CI: "true",
    APPLE_SIGNING_IDENTITY: "test-stable-identity",
    CARGO_ENCODED_RUSTFLAGS: "--remap-path-prefix=/test-home=/build-home",
    PATH: "/test-bin",
  });
  const result = tauriEnvironment("darwin", ["build", "--bundles", "app"], inherited);
  assert.deepEqual(result, {
    ...inherited,
    CI: "true",
    TAURI_BUNDLER_DMG_IGNORE_CI: "false",
  });
  assert.equal(inherited.CI, "false");
  assert.equal(inherited.TAURI_BUNDLER_DMG_IGNORE_CI, "true");
});

test("Windows and Linux environments are unchanged", () => {
  const inherited = { CI: "false", TAURI_BUNDLER_DMG_IGNORE_CI: "true", PATH: "test" };
  for (const platform of ["win32", "linux"]) {
    for (const command of ["build", "bundle"]) {
      assert.deepEqual(tauriEnvironment(platform, [command], inherited), inherited);
    }
  }
});

test("development, inspection, mobile and help commands are unchanged", () => {
  const inherited = { CI: "false", TAURI_BUNDLER_DMG_IGNORE_CI: "true" };
  for (const args of [["dev"], ["info"], ["--help"], [], ["ios", "build"], ["android", "build"]]) {
    assert.deepEqual(tauriEnvironment("darwin", args, inherited), inherited);
  }
});

test("CLI wrapper forwards arguments to the installed CLI without building", () => {
  const wrapper = fileURLToPath(new URL("./run-tauri.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [wrapper, "build", "--help"], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /--bundles/);
});

test("CLI wrapper preserves failure exit status", () => {
  const wrapper = fileURLToPath(new URL("./run-tauri.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [wrapper, "invalid-packaging-test-command"], { encoding: "utf8" });
  assert.equal(result.error, undefined);
  assert.notEqual(result.status, null);
  assert.notEqual(result.status, 0);
});
