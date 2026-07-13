#!/usr/bin/env node
// Build the package-local engine source and verify protocol, commit, lockfile,
// platform, and binary digest. Installation fails closed if identity cannot be
// established; no ambient PATH binary participates.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolveEngine } from "./engine-identity.mjs";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

try {
  const engine = resolveEngine({ rootPath: root });
  process.stdout.write(
    `receipts: verified bundled engine ${engine.identity.engine_version} ` +
      `(${engine.binarySha256.slice(0, 12)}…, protocol ${engine.identity.protocol_version})\n`,
  );
  process.stdout.write("receipts: run `receipts ready` to verify the pipeline.\n");
} catch (error) {
  process.stderr.write(`receipts: engine installation failed closed: ${error.message}\n`);
  process.exit(1);
}
