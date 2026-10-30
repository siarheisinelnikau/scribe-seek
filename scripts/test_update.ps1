param([string]$Executable = 'target/release/scribe-seek.exe')
$ErrorActionPreference = 'Stop'
# Installation now belongs to the sibling shared engine. Its finite native smoke
# fixture verifies process wait, replacement, rollback backup, restart and cleanup.
$taskManifest = Join-Path $PSScriptRoot '../../rust-auto-updator/Cargo.toml'
if (-not (Test-Path -LiteralPath $taskManifest)) { throw 'Provision the sibling rust-auto-updator workspace before testing updates.' }
& cargo run --manifest-path $taskManifest -p rust-auto-updator --example native-smoke --features smoke-test --locked -- --test-updater
if ($LASTEXITCODE -ne 0) { throw 'Shared native update smoke test failed' }
Write-Output 'Shared portable update engine smoke test passed.'
