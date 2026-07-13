# Agent Receipts one-shot installer for Windows PowerShell.
# Builds the Rust engine from this repo, installs the Node CLI, then
# proves the whole pipeline end to end with the readiness check.
$ErrorActionPreference = 'Stop'

function Require-Cmd($name, $hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        Write-Host "agent-receipts: missing required tool: $name" -ForegroundColor Red
        Write-Host "             install it from $hint, then re-run this script." -ForegroundColor Red
        exit 1
    }
}

Require-Cmd 'git'   'https://git-scm.com'
Require-Cmd 'cargo' 'https://rustup.rs'
Require-Cmd 'node'  'https://nodejs.org'
Require-Cmd 'npm'   'comes with Node'

$tmp = Join-Path $env:TEMP "agent-receipts-install-$([guid]::NewGuid().ToString('n').Substring(0,8))"

Write-Host 'agent-receipts: 1/3 building the Rust engine (receipts-core)...'
git clone --quiet --depth 1 https://github.com/inchwormz/agent-receipts $tmp
cargo install --quiet --path (Join-Path $tmp 'receipts-compiler')

Write-Host 'agent-receipts: 2/3 installing the Node CLI (receipts)...'
npm install -g --silent github:inchwormz/agent-receipts

Write-Host 'agent-receipts: 3/3 running readiness...'
receipts ready

Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

Write-Host ''
Write-Host 'agent-receipts: ready. Try:'
Write-Host '  receipts init .receipts/runs/my-run'
Write-Host '  receipts run --run-dir .receipts/runs/my-run --label test:hello -- echo hello'
