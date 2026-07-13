#!/usr/bin/env node
// Post-install hint: verify the `receipts-core` Rust binary is on PATH and print
// clear instructions if it's missing. We do NOT fail install on its absence —
// users may only want `ready` / `ingest` / `gate` (Node-only subset).
import { spawnSync } from "node:child_process";

const probe = spawnSync(process.platform === "win32" ? "where" : "which", ["receipts-core"], {
  stdio: "pipe",
  encoding: "utf8",
});

if (probe.status === 0) {
  const version = spawnSync("receipts-core", ["--version"], { stdio: "pipe", encoding: "utf8" });
  const v = version.status === 0 ? version.stdout.trim() : "receipts-core (installed)";
  process.stdout.write(`receipts: Rust compiler detected: ${v}\n`);
  process.stdout.write(`receipts: run \`receipts ready\` to verify the pipeline.\n`);
} else {
  process.stdout.write(
    `receipts: the Rust compiler \`receipts-core\` is NOT on PATH.\n` +
      `              Install it with:  cargo install --path receipts-compiler\n` +
      `              Without it, \`receipts compile\` will not work; \`ingest\`/\`gate\` remain usable.\n`
  );
}
