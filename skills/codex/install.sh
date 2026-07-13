#!/usr/bin/env sh
# Install Receipts as a Codex skill at ~/.codex/skills/receipts/.
set -eu
TARGET="${CODEX_HOME:-$HOME/.codex}/skills/receipts"
mkdir -p "$TARGET"
curl -fsSL https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/codex/SKILL.md -o "$TARGET/SKILL.md"
printf 'Codex skill installed at: %s\n' "$TARGET"
printf 'Next: install the `receipts` CLI:\n'
printf '  npm install -g github:inchwormz/agent-receipts\n'
