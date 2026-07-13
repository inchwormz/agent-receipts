import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function tempFixture(name) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `agent-receipts-${name}-`));
}

function compileIncompatibleEngine(dir) {
  const source = path.join(dir, "fake-engine.rs");
  const binary = path.join(dir, process.platform === "win32" ? "receipts.exe" : "receipts");
  fs.writeFileSync(
    source,
    [
      "fn main() {",
      "    if let Ok(marker) = std::env::var(\"RECEIPTS_FAKE_ENGINE_MARKER\") {",
      "        std::fs::write(marker, b\"used\").unwrap();",
      "    }",
      "}",
      "",
    ].join("\n"),
    "utf8",
  );
  const compiled = spawnSync("rustc", [source, "-o", binary], { encoding: "utf8" });
  assert.equal(compiled.status, 0, `fake engine compile failed: ${compiled.stderr}`);
  return binary;
}

test("CLI ignores an incompatible receipts binary first on PATH", (t) => {
  const fixture = tempFixture("wrong-path-engine");
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const fakeDir = path.join(fixture, "fake-bin");
  fs.mkdirSync(fakeDir, { recursive: true });
  compileIncompatibleEngine(fakeDir);

  const marker = path.join(fixture, "fake-engine-used.txt");
  const runDir = path.join(fixture, "run");
  const engineTarget = path.join(fixture, "verified-engine");
  const result = spawnSync(process.execPath, [path.join(repoRoot, "bin", "receipts.mjs"), "init", runDir], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${fakeDir}${path.delimiter}${process.env.PATH ?? ""}`,
      RECEIPTS_FAKE_ENGINE_MARKER: marker,
      RECEIPTS_ENGINE_TARGET_DIR: engineTarget,
    },
  });

  assert.equal(result.status, 0, `verified source engine must run: ${result.stderr}`);
  assert.equal(fs.existsSync(marker), false, "PATH-selected incompatible binary must never execute");
  assert.ok(fs.existsSync(path.join(runDir, "manifest.json")), "the verified engine must scaffold the run");
  assert.ok(fs.existsSync(path.join(engineTarget, "engine-manifest.json")), "engine digest/identity manifest must be recorded");
});

test("an explicitly supplied incompatible binary fails the identity handshake", (t) => {
  const fixture = tempFixture("explicit-wrong-engine");
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const fakeDir = path.join(fixture, "fake-bin");
  fs.mkdirSync(fakeDir, { recursive: true });
  const fake = compileIncompatibleEngine(fakeDir);
  const result = spawnSync(process.execPath, [path.join(repoRoot, "bin", "receipts.mjs"), "init", path.join(fixture, "run")], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      RECEIPTS_CORE_BINARY: fake,
      RECEIPTS_ENGINE_TARGET_DIR: path.join(fixture, "verified-engine"),
    },
  });

  assert.notEqual(result.status, 0, "incompatible explicit engine must be rejected");
  assert.match(result.stderr, /identity handshake/i);
});

test("doctor verifies engine, protected key, schemas, checks, and never guesses model metadata", (t) => {
  const fixture = tempFixture("doctor");
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const result = spawnSync(process.execPath, [path.join(repoRoot, "bin", "receipts.mjs"), "doctor"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      RECEIPTS_ENGINE_TARGET_DIR: path.join(fixture, "verified-engine"),
    },
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  const report = JSON.parse(result.stdout);
  assert.equal(report.ok, true);
  assert.match(report.identity.binary_digest, /^[0-9a-f]{64}$/);
  assert.match(report.identity.key_fingerprint, /^[0-9a-f]{64}$/);
  const byCode = new Map(report.checks.map((item) => [item.code, item.status]));
  assert.equal(byCode.get("installation_identity"), "pass");
  assert.equal(byCode.get("executor_key"), "pass");
  assert.equal(byCode.get("schema_support"), "pass");
  assert.equal(byCode.get("check_manifest"), "pass");
  assert.equal(byCode.get("model_runtime_metadata"), "unavailable");
  assert.equal(result.stdout.includes("secret_key"), false, "doctor must never print key material");
});
