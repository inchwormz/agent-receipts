#!/usr/bin/env sh
# Agent Receipts one-shot installer for macOS / Linux.
# Installs the npm dispatcher, which builds the bundled Rust engine, then
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

printf 'agent-receipts: 1/2 installing the dispatcher and bundled Rust engine...\n'
npm install -g --silent github:inchwormz/agent-receipts

printf 'agent-receipts: 2/2 running readiness...\n'
receipts ready

printf '\nagent-receipts: ready. Try:\n'
printf '  receipts init .receipts/runs/my-run\n'
printf '  receipts run --run-dir .receipts/runs/my-run --label test:hello -- echo hello\n'
