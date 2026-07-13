#!/usr/bin/env sh
# Agent Receipts one-shot installer for macOS / Linux.
# Builds the Rust engine from this repo, installs the Node CLI, then
# proves the whole pipeline end to end with the readiness check.
set -eu

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'agent-receipts: missing required tool: %s\n' "$1" >&2
    printf '             install it from %s, then re-run this script.\n' "$2" >&2
    exit 1
  fi
}

need git   'https://git-scm.com'
need cargo 'https://rustup.rs'
need npm   'https://nodejs.org'

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

printf 'agent-receipts: 1/3 building the Rust engine (receipts-core)...\n'
git clone --quiet --depth 1 https://github.com/inchwormz/agent-receipts "$tmpdir/agent-receipts"
cargo install --quiet --path "$tmpdir/agent-receipts/receipts-compiler"

printf 'agent-receipts: 2/3 installing the Node CLI (receipts)...\n'
npm install -g --silent github:inchwormz/agent-receipts

printf 'agent-receipts: 3/3 running readiness...\n'
receipts ready

printf '\nagent-receipts: ready. Try:\n'
printf '  receipts init .receipts/runs/my-run\n'
printf '  receipts run --run-dir .receipts/runs/my-run --label test:hello -- echo hello\n'
