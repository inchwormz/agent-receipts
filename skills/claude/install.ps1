# Install Receipts as a Claude Code skill at $env:USERPROFILE\.claude\skills\receipts\.
$ErrorActionPreference = 'Stop'
$claudeHome = if ($env:CLAUDE_HOME) { $env:CLAUDE_HOME } else { Join-Path $env:USERPROFILE '.claude' }
$target = Join-Path $claudeHome 'skills\receipts'
New-Item -ItemType Directory -Force -Path $target | Out-Null
Invoke-WebRequest -UseBasicParsing `
    -Uri 'https://raw.githubusercontent.com/inchwormz/mythos-skill/main/skills/claude/SKILL.md' `
    -OutFile (Join-Path $target 'SKILL.md')
Write-Host "Claude Code skill installed at: $target"
Write-Host 'Next: ensure receipts CLI is on PATH:'
Write-Host '  cargo install --path receipts-compiler  # from a repo checkout (crates.io publish pending)'
Write-Host '  npm install -g github:inchwormz/mythos-skill'
