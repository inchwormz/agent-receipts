import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("all install and package metadata names agent-receipts as the sole source", () => {
  const files = [
    "SKILL.md",
    "receipts-compiler/Cargo.toml",
    "receipts-compiler/README.md",
    "skills/claude/install.ps1",
    "skills/claude/install.sh",
    "skills/claude/SKILL.md",
    "skills/codex/install.ps1",
    "skills/codex/install.sh",
    "skills/codex/SKILL.md",
    "README.md",
    "driver.mjs",
    "bin/receipts.mjs",
    "bin/postinstall.mjs",
  ];
  const stale = [];
  for (const relative of files) {
    const text = fs.readFileSync(path.join(repoRoot, relative), "utf8");
    if (/mythos-skill|receipts-skill|cargo install receipts(?:\s|`|$)|cargo install --path receipts-compiler|receipts-core.*(?:on|from) PATH/i.test(text)) stale.push(relative);
  }
  assert.deepEqual(stale, [], `stale install sources remain in: ${stale.join(", ")}`);
});

test("npm package includes the complete bundled Rust engine source", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(repoRoot, "package.json"), "utf8"));
  const included = new Set(pkg.files ?? []);
  for (const required of [
    "receipts-compiler/Cargo.toml",
    "receipts-compiler/Cargo.lock",
    "receipts-compiler/build.rs",
    "receipts-compiler/build-source.json",
    "receipts-compiler/src/",
  ]) {
    assert.ok(included.has(required), `package files must include ${required}`);
  }
});

test("readiness pins Cargo to a repository-specific target and locked dependencies", () => {
  const source = fs.readFileSync(path.join(repoRoot, "scripts", "readiness.mjs"), "utf8");
  assert.match(source, /CARGO_TARGET_DIR:\s*readinessCargoTarget/);
  assert.match(source, /\["test",\s*"--locked"/);
  assert.doesNotMatch(source, /skipping cargo fmt\/test/);
});

test("CI never injects receipts-core through PATH and runs identity/install gates", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github", "workflows", "ci.yml"), "utf8");
  assert.doesNotMatch(workflow, /GITHUB_PATH|put receipts-core on PATH/);
  assert.match(workflow, /tests\/engine_identity\.test\.js/);
  assert.match(workflow, /tests\/install_sources\.test\.js/);
  assert.match(workflow, /cargo test --locked/);
});
