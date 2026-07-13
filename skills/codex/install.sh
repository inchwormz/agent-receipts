#!/usr/bin/env sh
# Install Receipts as a Codex skill at ~/.codex/skills/receipts/.
set -eu
TARGET="${CODEX_HOME:-$HOME/.codex}/skills/receipts"
mkdir -p "$TARGET"
curl -fsSL https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/codex/SKILL.md -o "$TARGET/SKILL.md"
printf 'Codex skill installed at: %s\n' "$TARGET"
printf 'Next: ensure `receipts` CLI is on PATH:\n'
printf '  cargo install --path receipts-compiler  # from a repo checkout (crates.io publish pending)\n'
printf '  npm install -g github:inchwormz/mythos-skill\n'
