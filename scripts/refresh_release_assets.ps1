# Refresh explicitly requested release names and documentation without rebuilding
# or changing the already published executable.
$ErrorActionPreference = 'Stop'
if (-not $env:GITHUB_TOKEN -or -not $env:GITHUB_REPOSITORY -or $env:GITHUB_EVENT_NAME -ne 'push' -or $env:GITHUB_REF -ne 'refs/heads/main') { throw 'A GitHub Actions main-push run is required' }
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo version' }
$version = ($metadata.packages | Where-Object name -eq 'scribe-seek').version
$tag = "v$version"
$trailer = '(?m)^Refresh-Release-Assets: ' + [regex]::Escape($tag) + '\r?$'
if (-not $version -or $env:RELEASE_COMMIT_MESSAGE -notmatch $trailer) { throw 'An explicit version-matched asset-refresh trailer is required' }
$headers = @{ Authorization = "Bearer $env:GITHUB_TOKEN"; Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2026-03-10' }
$api = "https://api.github.com/repos/$env:GITHUB_REPOSITORY"
$release = Invoke-RestMethod -Uri "$api/releases/tags/$tag" -Headers $headers
if ($release.draft -or $release.prerelease -or $release.tag_name -ne $tag) { throw 'A published stable version is required' }
$exeName = "ScribeSeek-$tag.exe"
$zipName = "ScribeSeek-$tag.zip"
$legacyName = 'scribe-seek-windows-x64.exe'
$source = @($release.assets | Where-Object name -eq $exeName)
if ($source.Count -eq 0) { $source = @($release.assets | Where-Object name -eq $legacyName) }
if ($source.Count -ne 1 -or $source[0].digest -notmatch '^sha256:[0-9a-f]{64}$' -or $source[0].size -le 0 -or $source[0].size -gt 268435456) { throw 'Missing verified source executable' }
$source = $source[0]
$expectedUrl = "https://github.com/$env:GITHUB_REPOSITORY/releases/download/$tag/$($source.name)"
if ($source.browser_download_url -ne $expectedUrl) { throw 'Source executable URL is not from this repository' }
$working = Join-Path $env:RUNNER_TEMP ('release-refresh-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $working | Out-Null
$sourcePath = Join-Path $working 'published.exe'
Invoke-WebRequest -Uri $source.browser_download_url -OutFile $sourcePath
$digest = 'sha256:' + (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLower()
if ($digest -ne $source.digest -or (Get-Item -LiteralPath $sourcePath).Length -ne $source.size) { throw 'Published executable verification failed' }
& "$PSScriptRoot/package_release.ps1" -Executable $sourcePath -OutputDirectory 'release-assets'
$notes = Get-Content -LiteralPath (Join-Path $PSScriptRoot "../releases/$tag.md") -Raw -Encoding utf8
foreach ($name in @($exeName, $legacyName, $zipName, 'SHA256SUMS.txt')) {
    $path = Join-Path 'release-assets' $name
    $expectedDigest = 'sha256:' + (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
    $asset = $release.assets | Where-Object name -eq $name
    if ($asset -and $asset.digest -ne $expectedDigest) {
        if ($name -in @($exeName, $legacyName)) { throw 'Asset refresh cannot change a published executable' }
        Invoke-RestMethod -Method Delete -Uri "$api/releases/assets/$($asset.id)" -Headers $headers | Out-Null
        $asset = $null
    }
    if (-not $asset) {
        $encodedName = [Uri]::EscapeDataString($name)
        $upload = "https://uploads.github.com/repos/$env:GITHUB_REPOSITORY/releases/$($release.id)/assets?name=$encodedName"
        $asset = Invoke-RestMethod -Method Post -Uri $upload -Headers $headers -InFile $path -ContentType 'application/octet-stream'
    }
    if ($asset.digest -ne $expectedDigest) { throw "GitHub asset digest did not match: $name" }
    if ($name -eq $legacyName) {
        $label = @{ label = 'Compatibility copy for existing in-app updaters' } | ConvertTo-Json
        Invoke-RestMethod -Method Patch -Uri "$api/releases/assets/$($asset.id)" -Headers $headers -Body $label -ContentType 'application/json' | Out-Null
    }
}
# The versioned archive is uploaded and verified before the obsolete archive name
# is removed. The legacy EXE remains available for existing installations.
foreach ($obsolete in @($release.assets | Where-Object name -eq 'scribe-seek-windows-x64.zip')) {
    Invoke-RestMethod -Method Delete -Uri "$api/releases/assets/$($obsolete.id)" -Headers $headers | Out-Null
}
$body = @{ name = "Scribe Seek $tag"; body = $notes } | ConvertTo-Json
$updated = Invoke-RestMethod -Method Patch -Uri "$api/releases/$($release.id)" -Headers $headers -Body $body -ContentType 'application/json'
if ($updated.tag_name -ne $tag -or $updated.draft) { throw 'Release tag changed during asset refresh' }
Write-Output "Updated $($updated.html_url); executable bytes and legacy update compatibility were preserved."
