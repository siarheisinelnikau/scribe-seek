# Prefer Visual Studio's matching CMake (older PATH versions may not know VS 2026).
# Usage: . ./scripts/build-env.ps1
$ErrorActionPreference = 'Stop'
$localClang = Join-Path $PSScriptRoot '..\.test-data\tooling\clang\native'
if (-not $env:LIBCLANG_PATH -and (Test-Path -LiteralPath (Join-Path $localClang 'libclang.dll'))) {
    $env:LIBCLANG_PATH = (Resolve-Path -LiteralPath $localClang).Path
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not $env:CMAKE -and (Test-Path -LiteralPath $vswhere)) {
    $vsPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vsPath) {
        $bundledCmake = Join-Path $vsPath 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
        if (Test-Path -LiteralPath $bundledCmake) { $env:CMAKE = $bundledCmake }
    }
}
