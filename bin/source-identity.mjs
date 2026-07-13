#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const output = path.join(root, "receipts-compiler", "build-source.json");

if (process.argv.includes("--clean")) {
  fs.rmSync(output, { force: true });
  process.exit(0);
}

const commitResult = spawnSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" });
const commit = (commitResult.stdout ?? "").trim().toLowerCase();
if (commitResult.status !== 0 || !/^[0-9a-f]{40}$/.test(commit)) {
  process.stderr.write(`receipts prepack: cannot resolve public commit: ${(commitResult.stderr ?? "").trim()}\n`);
  process.exit(1);
}
const lock = fs.readFileSync(path.join(root, "receipts-compiler", "Cargo.lock"));
const dependencyLockDigest = crypto.createHash("sha256").update(lock).digest("hex");
fs.writeFileSync(
  output,
  `${JSON.stringify({ build_commit: commit, dependency_lock_digest: dependencyLockDigest }, null, 2)}\n`,
  "utf8",
);
