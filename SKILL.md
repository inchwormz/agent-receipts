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
curl -fsSL https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/claude/install.sh | sh

# Codex
curl -fsSL https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/codex/install.sh | sh
```

PowerShell:

```powershell
# Claude Code
iwr https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/claude/install.ps1 | iex

# Codex
iwr https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/codex/install.ps1 | iex
```

Either skill needs the Receipts runtime on PATH (from a repo checkout until the crates.io/npm publishes land):

```bash
cargo install --path receipts-compiler   # installs the receipts-core engine
npm install -g github:inchwormz/mythos-skill   # installs the `receipts` CLI
receipts ready
```

(Repository URLs still say `mythos-skill` — the GitHub rename to Receipts is pending; these URLs will keep working via GitHub redirects after it.)

See [README.md](./README.md) for the full mental model and runtime architecture.
