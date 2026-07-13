#!/usr/bin/env sh
# Install Receipts as a Claude Code skill at ~/.claude/skills/receipts/.
set -eu
TARGET="${CLAUDE_HOME:-$HOME/.claude}/skills/receipts"
mkdir -p "$TARGET"
curl -fsSL https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/claude/SKILL.md -o "$TARGET/SKILL.md"
printf 'Claude Code skill installed at: %s\n' "$TARGET"
printf 'Next: ensure `receipts` CLI is on PATH:\n'
printf '  cargo install --path receipts-compiler  # from a repo checkout (crates.io publish pending)\n'
printf '  npm install -g github:inchwormz/mythos-skill\n'
