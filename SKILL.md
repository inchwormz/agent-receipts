---
name: receipts
description: Router pointing at the Claude Code and Codex skill packages. Install one of those; this file is a pointer, not a skill.
---

# Receipts — skill router

Receipts has two Prime surfaces and each installs to a different home directory:

- **Claude Code** — `skills/claude/SKILL.md` installs to `~/.claude/skills/receipts/SKILL.md`.
- **Codex** — `skills/codex/SKILL.md` installs to `~/.codex/skills/receipts/SKILL.md`.

Install the one that matches your Prime surface:

```bash
# Claude Code
curl -fsSL https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/claude/install.sh | sh

# Codex
curl -fsSL https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/codex/install.sh | sh
```

PowerShell:

```powershell
# Claude Code
iwr https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/claude/install.ps1 | iex

# Codex
iwr https://raw.githubusercontent.com/inchwormz/agent-receipts/main/skills/codex/install.ps1 | iex
```

Either skill needs the Receipts runtime from the canonical public repository:

```bash
npm install -g github:inchwormz/agent-receipts
receipts ready
```

See [README.md](./README.md) for the full mental model and runtime architecture.
