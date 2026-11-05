import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const runner = fileURLToPath(new URL("./replace-macos-local.sh", import.meta.url));
const macTest = (name, fn) => test(name, { skip: process.platform !== "darwin" }, fn);

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "boltscribe action test "));
  const scripts = join(root, "scripts");
  const bin = join(root, "bin");
  mkdirSync(scripts);
  mkdirSync(bin);
  copyFileSync(runner, join(scripts, "replace-macos-local.sh"));
  symlinkSync(process.execPath, join(bin, "node"));
  for (const tool of ["npm", "cargo"]) {
    writeFileSync(join(bin, tool), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  }
  writeFileSync(join(scripts, "replace-macos-local.mjs"), `
    console.log('stdout:' + process.env.ACTION_TEST_ID);
    console.error('stderr:' + process.env.ACTION_TEST_ID);
    console.log('cwd:' + process.cwd());
    console.log('args:' + JSON.stringify(process.argv.slice(2)));
    process.exit(Number(process.env.ACTION_TEST_EXIT));
  `);
  return {
    root,
    log: join(root, ".codex/logs/package-and-replace-action.log"),
    run(args = [], exitCode = 0, runId = "test") {
      return spawnSync("/bin/bash", [join(scripts, "replace-macos-local.sh"), ...args], {
        cwd: tmpdir(),
        env: { ...process.env, PATH: `${bin}:/usr/bin:/bin`, ACTION_TEST_EXIT: String(exitCode), ACTION_TEST_ID: runId },
        input: "", encoding: "utf8", timeout: 5000,
      });
    },
  };
}

macTest("action logs startup, stdout, stderr and completion from another working directory", () => {
  const app = fixture();
  const result = app.run(["--check", "argument with spaces"]);
  assert.equal(result.status, 0, result.stderr);
  const log = readFileSync(app.log, "utf8");
  assert.match(log, /开始 BoltScribe/);
  assert.match(log, /stdout:test/);
  assert.match(log, /stderr:test/);
  assert.ok(log.includes(`cwd:${realpathSync(app.root)}`));
  assert.ok(log.includes('args:["--check","argument with spaces"]'));
  assert.match(log, /退出码 0/);
  assert.equal(statSync(app.log).mode & 0o777, 0o600);
});

macTest("action returns the child failure code and retains its diagnostic output", () => {
  const app = fixture();
  const result = app.run(["--check"], 37, "failure");
  assert.equal(result.status, 37);
  const log = readFileSync(app.log, "utf8");
  assert.match(log, /stderr:failure/);
  assert.match(log, /退出码 37/);
  assert.match(result.stdout, /stderr:failure/);
  assert.match(result.stderr, /操作失败/);
});

macTest("successive actions append logs instead of losing earlier failures", () => {
  const app = fixture();
  assert.equal(app.run([], 12, "first").status, 12);
  const first = readFileSync(app.log, "utf8");
  assert.equal(app.run([], 0, "second").status, 0);
  const log = readFileSync(app.log, "utf8");
  assert.ok(log.startsWith(first));
  assert.match(log, /stdout:second/);
  assert.match(log, /退出码 12/);
  assert.match(log, /退出码 0/);
});

macTest("keep-open is consumed and does not hang a noninteractive action", () => {
  const app = fixture();
  const result = app.run(["--keep-open", "--check"], 23);
  assert.equal(result.status, 23, result.stderr);
  const log = readFileSync(app.log, "utf8");
  assert.ok(log.includes('args:["--check"]'));
  assert.doesNotMatch(log, /--keep-open/);
});
