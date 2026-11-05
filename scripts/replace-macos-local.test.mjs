import assert from "node:assert/strict";
import test from "node:test";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { assertCompatibleApp, assertInstalledCopy, command, installApp, selectSigningIdentity } from "./replace-macos-local.mjs";

test("failed commands preserve stderr and their original exit code", () => {
  assert.throws(() => command(process.execPath, ["-e", "console.error('signature check failed'); process.exit(23)"]), (error) => {
    assert.equal(error.exitCode, 23);
    assert.match(error.message, /signature check failed/);
    assert.match(error.message, /退出码 23/);
    return true;
  });
});

test("a missing executable reports the spawn error", () => {
  assert.throws(() => command("/nonexistent/boltscribe-packaging-tool", []), (error) => {
    assert.match(error.message, /ENOENT/);
    assert.equal(error.exitCode, 1);
    return true;
  });
});

const authority = "Apple Development: Packaging Test";
const identity = "A".repeat(40);
const signature = {
  authority,
  team: "TESTTEAM",
  requirement: 'identifier "cn.local.boltscribe" and anchor apple generic',
  bundleId: "cn.local.boltscribe",
  arch: "arm64",
  version: "1.6.0",
  hash: "test-executable-sha256",
};

test("selects the exact installed signing authority without depending on identity order", () => {
  const output = `  1) ${"B".repeat(40)} "Apple Development: Other Test"\n  2) ${identity} "${authority}"\n     2 valid identities found`;
  assert.equal(selectSigningIdentity(output, authority), identity);
});

test("rejects missing, expired and ambiguous signing identities", () => {
  for (const output of [
    "0 valid identities found",
    `1) ${identity} "${authority}" (CSSMERR_TP_CERT_EXPIRED)`,
    `1) ${identity} "${authority}"\n2) ${"B".repeat(40)} "${authority}"`,
  ]) {
    assert.throws(() => selectSigningIdentity(output, authority), /唯一匹配/);
  }
  for (const unsupported of [undefined, "-", "Developer ID Application: Test"]) {
    assert.throws(() => selectSigningIdentity("", unsupported), /Apple Development/);
  }
});

test("allows a new version and executable while preserving signing lineage and platform", () => {
  assert.doesNotThrow(() => assertCompatibleApp(signature, {
    ...signature, version: "1.7.0", hash: "updated-executable-sha256",
  }));
});

test("rejects signing lineage, bundle identity and architecture mismatches", () => {
  for (const field of ["authority", "team", "requirement", "bundleId", "arch"]) {
    assert.throws(
      () => assertCompatibleApp(signature, { ...signature, [field]: "different" }),
      new RegExp(field),
    );
  }
});

test("installed copy must match both the built version and executable hash", () => {
  assert.doesNotThrow(() => assertInstalledCopy(signature, { ...signature }));
  for (const field of ["version", "hash"]) {
    assert.throws(
      () => assertInstalledCopy(signature, { ...signature, [field]: "different" }),
      /SHA-256/,
    );
  }
  assert.throws(
    () => assertInstalledCopy(signature, { ...signature, team: "other-team" }),
    /team/,
  );
});

function installationFixture() {
  // Keep tiny fixtures in the OS temporary directory; bundles are never deleted.
  const root = mkdtempSync(join(tmpdir(), "boltscribe-replacement-test-"));
  const source = join(root, "built", "BoltScribe.app");
  const destination = join(root, "Applications", "BoltScribe.app");
  const trashDir = join(root, "Trash");
  for (const [app, contents] of [[source, "new-app"], [destination, "old-app"]]) {
    mkdirSync(app, { recursive: true });
    writeFileSync(join(app, "executable"), contents);
  }
  return { source, destination, trashDir };
}

function appContents(app) {
  return readFileSync(join(app, "executable"), "utf8");
}

function copyApp(command, [source, destination]) {
  assert.equal(command, "/usr/bin/ditto");
  cpSync(source, destination, { recursive: true });
}

test("successful installation keeps the old app in Trash and verifies the installed copy", () => {
  const fixture = installationFixture();
  let verified = false;
  const backup = installApp({
    ...fixture,
    copy: copyApp,
    verify: (destination) => {
      assert.equal(destination, fixture.destination);
      assert.equal(appContents(destination), "new-app");
      verified = true;
    },
  });
  assert.equal(verified, true);
  assert.equal(appContents(backup), "old-app");
  assert.equal(appContents(fixture.destination), "new-app");
  assert.equal(appContents(fixture.source), "new-app");
  assert.equal(readdirSync(fixture.trashDir).length, 1);
});

test("copy failure before creating a destination restores the old app", () => {
  const fixture = installationFixture();
  const failure = new Error("copy failed");
  assert.throws(() => installApp({
    ...fixture,
    copy: () => { throw failure; },
    verify: () => assert.fail("verification cannot run after a copy failure"),
  }), (error) => error.message.includes("已恢复旧应用") && error.cause === failure);
  assert.equal(appContents(fixture.destination), "old-app");
  assert.equal(appContents(fixture.source), "new-app");
  assert.deepEqual(readdirSync(fixture.trashDir), []);
});

test("partial copy failure restores the old app and moves the partial app to Trash", () => {
  const fixture = installationFixture();
  assert.throws(() => installApp({
    ...fixture,
    copy: (_command, [_source, destination]) => {
      mkdirSync(destination);
      writeFileSync(join(destination, "executable"), "partial-app");
      throw new Error("copy interrupted");
    },
    verify: () => assert.fail("verification cannot run after a copy failure"),
  }), /已恢复旧应用/);
  assert.equal(appContents(fixture.destination), "old-app");
  const failedCopies = readdirSync(fixture.trashDir);
  assert.equal(failedCopies.length, 1);
  assert.match(failedCopies[0], /^BoltScribe-failed-/);
  assert.equal(appContents(join(fixture.trashDir, failedCopies[0])), "partial-app");
});

test("verification failure restores the old app and preserves the rejected copy", () => {
  const fixture = installationFixture();
  const failure = new Error("signature mismatch");
  assert.throws(() => installApp({
    ...fixture,
    copy: copyApp,
    verify: () => { throw failure; },
  }), (error) => error.message.includes("已恢复旧应用") && error.cause === failure);
  assert.equal(appContents(fixture.destination), "old-app");
  const rejectedCopies = readdirSync(fixture.trashDir);
  assert.equal(rejectedCopies.length, 1);
  assert.equal(appContents(join(fixture.trashDir, rejectedCopies[0])), "new-app");
  assert.equal(existsSync(fixture.source), true);
});
