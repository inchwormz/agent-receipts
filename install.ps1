# Agent Receipts one-shot installer for Windows PowerShell.
# Installs the npm dispatcher, which builds the bundled Rust engine, then
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

Write-Host 'agent-receipts: 1/2 installing the dispatcher and bundled Rust engine...'
npm install -g --silent github:inchwormz/agent-receipts

Write-Host 'agent-receipts: 2/2 running readiness...'
receipts ready

Write-Host ''
Write-Host 'agent-receipts: ready. Try:'
Write-Host '  receipts init .receipts/runs/my-run'
Write-Host '  receipts run --run-dir .receipts/runs/my-run --label test:hello -- echo hello'
