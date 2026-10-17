param(
    [string]$Executable = 'target/release/scribe-seek.exe',
    [string]$OutputDirectory = 'dist'
)
$ErrorActionPreference = 'Stop'
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo version' }
$version = ($metadata.packages | Where-Object name -eq 'scribe-seek').version
if (-not $version) { throw 'Missing application version' }
$exeName = "ScribeSeek-v$version.exe"
$zipName = "ScribeSeek-v$version.zip"
$stage = Join-Path $PSScriptRoot ("../.test-data/package-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $stage,$OutputDirectory | Out-Null
Copy-Item -LiteralPath $Executable -Destination (Join-Path $stage $exeName)
Copy-Item README.md,LICENSE,THIRD_PARTY_LICENSES.txt,locales/UNICODE_LICENSE.txt -Destination $stage
New-Item -ItemType Directory -Path (Join-Path $stage 'docs') | Out-Null
Copy-Item -LiteralPath docs/screenshots -Destination (Join-Path $stage 'docs') -Recurse
$hash = (Get-FileHash -LiteralPath (Join-Path $stage $exeName) -Algorithm SHA256).Hash.ToLower()
"$hash  $exeName" | Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding ascii
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $OutputDirectory $zipName) -Force
Copy-Item -LiteralPath (Join-Path $stage $exeName),(Join-Path $stage 'SHA256SUMS.txt') -Destination $OutputDirectory -Force
# Older installed versions request this exact asset name. It is the same x64 build.
Copy-Item -LiteralPath (Join-Path $stage $exeName) -Destination (Join-Path $OutputDirectory 'scribe-seek-windows-x64.exe') -Force
Write-Output "Packaged $exeName and $zipName; retained the legacy updater alias."
