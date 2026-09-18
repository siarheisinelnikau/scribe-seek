param([string]$Executable = 'target/release/scribe-seek.exe')
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Executable).Path
$testRoot = Join-Path (Split-Path $PSScriptRoot -Parent) ('.test-data/update-smoke-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$fixtureSource = Join-Path $testRoot 'fixture.rs'
@'
#![windows_subsystem = "windows"]
fn main() {
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, env!("UPDATE_SMOKE_CONTENT")).unwrap();
    }
}
'@ | Set-Content -LiteralPath $fixtureSource -Encoding utf8
$targetPath = Join-Path $testRoot 'app.exe'
$stagedPath = Join-Path $testRoot 'new.exe'
$backupPath = Join-Path $testRoot 'previous.exe'
$helperPath = Join-Path $testRoot 'helper.exe'
$markerPath = Join-Path $testRoot 'result.txt'
$planPath = Join-Path $testRoot 'plan.json'
$previousContent = $env:UPDATE_SMOKE_CONTENT
try {
    $env:UPDATE_SMOKE_CONTENT = 'original'
    & rustc $fixtureSource --crate-name update_fixture -C opt-level=s -C target-feature=+crt-static -o $targetPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not build original update fixture' }
    $env:UPDATE_SMOKE_CONTENT = 'updated'
    & rustc $fixtureSource --crate-name update_fixture -C opt-level=s -C target-feature=+crt-static -o $stagedPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not build update fixture' }
} finally { $env:UPDATE_SMOKE_CONTENT = $previousContent }
$originalHash = (Get-FileHash -LiteralPath $targetPath -Algorithm SHA256).Hash.ToLower()
$newHash = (Get-FileHash -LiteralPath $stagedPath -Algorithm SHA256).Hash.ToLower()
Copy-Item -LiteralPath $exePath -Destination $helperPath
@{
    target = $targetPath; staged = $stagedPath; backup = $backupPath; sha256 = $newHash
    parent_pid = 0; data_dir = $testRoot; source = $markerPath; locale = 'en'
} | ConvertTo-Json | Set-Content -LiteralPath $planPath -Encoding utf8NoBOM
$startInfo = [System.Diagnostics.ProcessStartInfo]::new($helperPath)
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
$startInfo.ArgumentList.Add('--apply-update')
$startInfo.ArgumentList.Add($planPath)
$helperProcess = [System.Diagnostics.Process]::Start($startInfo)
if (-not $helperProcess.WaitForExit(10000)) { $helperProcess.Kill(); throw 'Update helper did not finish' }
$deadline = [DateTime]::UtcNow.AddSeconds(5)
while (-not (Test-Path -LiteralPath $markerPath) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
if (-not (Test-Path -LiteralPath $markerPath) -or (Get-Content -LiteralPath $markerPath -Raw) -ne 'updated') { throw 'Updated application did not restart' }
if ((Get-FileHash -LiteralPath $targetPath -Algorithm SHA256).Hash.ToLower() -ne $newHash) { throw 'Wrong update installed' }
if ((Get-FileHash -LiteralPath $backupPath -Algorithm SHA256).Hash.ToLower() -ne $originalHash) { throw 'Original backup was not preserved' }
Write-Output 'Native update helper: verified replacement, restart and original backup passed.'
