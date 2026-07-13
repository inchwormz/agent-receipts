#!/usr/bin/env sh
# Install Receipts as a Claude Code skill at ~/.claude/skills/receipts/.
set -eu
TARGET="${CLAUDE_HOME:-$HOME/.claude}/skills/receipts"
mkdir -p "$TARGET"
curl -fsSL https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/claude/SKILL.md -o "$TARGET/SKILL.md"
printf 'Claude Code skill installed at: %s\n' "$TARGET"
printf 'Next: install the `receipts` CLI:\n'
printf '  npm install -g github:inchwormz/agent-receipts\n'
