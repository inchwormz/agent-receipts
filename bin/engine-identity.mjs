import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const ENGINE_PROTOCOL_VERSION = "1";

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function canonicalJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function sourceCommit(rootPath, env) {
  if (/^[0-9a-f]{40}$/i.test(env.RECEIPTS_BUILD_COMMIT ?? "")) {
    return env.RECEIPTS_BUILD_COMMIT.toLowerCase();
  }
  const result = spawnSync("git", ["-C", rootPath, "rev-parse", "HEAD"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  const commit = (result.stdout ?? "").trim();
  if (result.status === 0 && /^[0-9a-f]{40}$/i.test(commit)) {
    return commit.toLowerCase();
  }
  const packagedIdentity = path.join(rootPath, "receipts-compiler", "build-source.json");
  if (fs.existsSync(packagedIdentity)) {
    const packaged = JSON.parse(fs.readFileSync(packagedIdentity, "utf8"));
    if (/^[0-9a-f]{40}$/i.test(packaged.build_commit ?? "")) {
      return packaged.build_commit.toLowerCase();
    }
  }
  throw new Error(`identity handshake cannot resolve build commit: ${(result.stderr ?? "").trim() || "not a Git checkout"}`);
}

function expectedSourceIdentity(rootPath, env) {
  const lockPath = path.join(rootPath, "receipts-compiler", "Cargo.lock");
  if (!fs.existsSync(lockPath)) {
    throw new Error(`identity handshake cannot find dependency lock: ${lockPath}`);
  }
  const identity = {
    protocol_version: ENGINE_PROTOCOL_VERSION,
    build_commit: sourceCommit(rootPath, env),
    dependency_lock_digest: sha256File(lockPath),
  };
  const packagedIdentity = path.join(rootPath, "receipts-compiler", "build-source.json");
  if (fs.existsSync(packagedIdentity)) {
    const packaged = JSON.parse(fs.readFileSync(packagedIdentity, "utf8"));
    if (packaged.dependency_lock_digest !== identity.dependency_lock_digest) {
      throw new Error("identity handshake packaged dependency-lock digest does not match Cargo.lock");
    }
  }
  return identity;
}

function readEngineIdentity(binaryPath) {
  const result = spawnSync(binaryPath, ["identity"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error || result.status !== 0) {
    const detail = result.error?.message || (result.stderr ?? "").trim() || `exit ${result.status}`;
    throw new Error(
      `identity handshake command failed for ${binaryPath}: ${detail}`,
    );
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`identity handshake returned invalid JSON for ${binaryPath}: ${error.message}`);
  }
}

function validateIdentity(identity, expected) {
  for (const field of ["protocol_version", "build_commit", "dependency_lock_digest"]) {
    if (identity[field] !== expected[field]) {
      throw new Error(
        `identity handshake ${field} mismatch: expected ${expected[field]}, got ${identity[field] ?? "missing"}`,
      );
    }
  }
  const expectedOs = { win32: "windows", darwin: "macos" }[process.platform] ?? process.platform;
  const expectedArch = { x64: "x86_64", arm64: "aarch64" }[process.arch] ?? process.arch;
  if (identity.os !== expectedOs || identity.arch !== expectedArch) {
    throw new Error(
      `identity handshake platform mismatch: expected ${expectedOs}/${expectedArch}, got ${identity.os ?? "missing"}/${identity.arch ?? "missing"}`,
    );
  }
}

function verifyManifest(binaryPath, manifestPath, expected) {
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`identity handshake manifest is missing: ${manifestPath}`);
  }
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  const digest = sha256File(binaryPath);
  if (manifest.binary_sha256 !== digest) {
    throw new Error(
      `identity handshake binary digest mismatch: expected ${manifest.binary_sha256 ?? "missing"}, got ${digest}`,
    );
  }
  const identity = readEngineIdentity(binaryPath);
  validateIdentity(identity, expected);
  if (JSON.stringify(manifest.identity) !== JSON.stringify(identity)) {
    throw new Error("identity handshake manifest metadata does not match the executable");
  }
  return { binaryPath, manifestPath, binarySha256: digest, identity };
}

function platformBinaryName() {
  return process.platform === "win32" ? "receipts.exe" : "receipts";
}

function sourceEngine(rootPath, env) {
  const cargoManifest = path.join(rootPath, "receipts-compiler", "Cargo.toml");
  if (!fs.existsSync(cargoManifest)) {
    throw new Error("identity handshake cannot find bundled Rust engine source");
  }
  const targetDir = path.resolve(
    env.RECEIPTS_ENGINE_TARGET_DIR ?? path.join(rootPath, ".receipts", "engine", `${process.platform}-${process.arch}`),
  );
  fs.mkdirSync(targetDir, { recursive: true });
  const expected = expectedSourceIdentity(rootPath, env);
  const build = spawnSync(
    "cargo",
    ["build", "--locked", "--release", "--manifest-path", cargoManifest, "--bin", "receipts"],
    {
      cwd: rootPath,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...env,
        CARGO_TARGET_DIR: targetDir,
        RECEIPTS_BUILD_COMMIT: expected.build_commit,
        RECEIPTS_LOCK_DIGEST: expected.dependency_lock_digest,
      },
    },
  );
  if (build.error || build.status !== 0) {
    const detail = build.error?.message || (build.stderr ?? "").trim() || `exit ${build.status}`;
    throw new Error(
      `identity handshake could not build bundled engine: ${detail}`,
    );
  }
  const binaryPath = path.join(targetDir, "release", platformBinaryName());
  if (!fs.existsSync(binaryPath)) {
    throw new Error(`identity handshake build did not produce ${binaryPath}`);
  }
  const identity = readEngineIdentity(binaryPath);
  validateIdentity(identity, expected);
  const manifestPath = path.join(targetDir, "engine-manifest.json");
  const manifest = {
    manifest_version: 1,
    binary_sha256: sha256File(binaryPath),
    identity,
  };
  fs.writeFileSync(manifestPath, canonicalJson(manifest), "utf8");
  return verifyManifest(binaryPath, manifestPath, expected);
}

export function resolveEngine({ rootPath, env = process.env } = {}) {
  if (!rootPath) throw new Error("identity handshake requires the package root");
  if (env.RECEIPTS_CORE_BINARY) {
    const binaryPath = path.resolve(env.RECEIPTS_CORE_BINARY);
    const manifestPath = env.RECEIPTS_ENGINE_MANIFEST
      ? path.resolve(env.RECEIPTS_ENGINE_MANIFEST)
      : "";
    if (!fs.existsSync(binaryPath)) {
      throw new Error(`identity handshake explicit binary is missing: ${binaryPath}`);
    }
    if (!manifestPath) {
      throw new Error("identity handshake explicit binary requires RECEIPTS_ENGINE_MANIFEST");
    }
    return verifyManifest(binaryPath, manifestPath, expectedSourceIdentity(rootPath, env));
  }
  return sourceEngine(rootPath, env);
}
