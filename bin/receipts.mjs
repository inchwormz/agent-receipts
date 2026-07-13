#!/usr/bin/env node
// npm dispatcher only. All run-data interpretation, trust classification,
// composites, gates, reports, and readiness live in the Rust engine.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs";
import { resolveEngine } from "./engine-identity.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.dirname(here);
const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));

const COMMANDS = [
  "init",
  "run",
  "check",
  "ingest",
  "absorb",
  "conclude",
  "diff",
  "resolve",
  "compile",
  "synthesize",
  "gate",
  "report",
  "next",
  "doctor",
  "project-public",
  "ready",
  "identity",
  "session",
  "adjudicate",
  "import-eval",
];

function printHelp() {
  process.stdout.write(
    `receipts ${pkg.version} — local-first proof for AI agent work\n\n` +
      `USAGE:\n    receipts <COMMAND> [ARGS]\n\n` +
      `COMMANDS:\n${COMMANDS.map((name) => `    ${name}`).join("\n")}\n\n` +
      `Run \`receipts ready\` to exercise the installed Rust engine end to end.\n`,
  );
}

const [, , command = "help", ...args] = process.argv;
if (["help", "--help", "-h"].includes(command)) {
  printHelp();
  process.exit(0);
}
if (["version", "--version", "-V"].includes(command)) {
  process.stdout.write(`receipts ${pkg.version}\n`);
  process.exit(0);
}
if (!COMMANDS.includes(command)) {
  process.stderr.write(`receipts: unknown command \`${command}\` — try \`receipts help\`\n`);
  process.exit(2);
}

let core;
try {
  core = resolveEngine({ rootPath: root }).binaryPath;
} catch (error) {
  process.stderr.write(`receipts: engine identity handshake failed: ${error.message}\n`);
  process.exit(1);
}
const result = spawnSync(core, [command, ...args], { stdio: "inherit" });
if (result.error) {
  process.stderr.write(`receipts: failed to launch verified engine: ${result.error.message}\n`);
  process.exit(1);
}
process.exit(result.status ?? 1);
