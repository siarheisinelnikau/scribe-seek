# Runs after a successful Windows build. A new Cargo version on main creates
# its version tag and release; subsequent builds leave published versions alone.
$ErrorActionPreference = 'Stop'
$isVersionTag = $env:GITHUB_REF_TYPE -eq 'tag'
$isMainPush = $env:GITHUB_EVENT_NAME -eq 'push' -and $env:GITHUB_REF -eq 'refs/heads/main'
if (-not $env:GITHUB_TOKEN -or -not $env:GITHUB_REPOSITORY -or -not ($isVersionTag -or $isMainPush)) { throw 'A GitHub Actions main-push or version-tag run is required' }
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo version' }
$version = ($metadata.packages | Where-Object name -eq 'scribe-seek').version
if (-not $version) { throw 'Missing application version' }
$tag = "v$version"
if ($isVersionTag -and $env:GITHUB_REF_NAME -ne $tag) { throw 'Version tag must match Cargo.toml' }
$headers = @{
    Authorization = "Bearer $env:GITHUB_TOKEN"
    Accept = 'application/vnd.github+json'
    'X-GitHub-Api-Version' = '2026-03-10'
}
$releasesUrl = "https://api.github.com/repos/$env:GITHUB_REPOSITORY/releases"
$release = $null
try {
    $release = Invoke-RestMethod -Uri "$releasesUrl/tags/$tag" -Headers $headers
} catch {
    if ([int]$_.Exception.Response.StatusCode -ne 404) { throw }
}
$replaceCommit = $null
if ($env:RELEASE_COMMIT_MESSAGE -match '(?m)^Replace-Release-Commit: ([0-9a-f]{40})\r?$') { $replaceCommit = $Matches[1] }
$repairRelease = $release -and $replaceCommit -and $release.target_commitish -eq $replaceCommit -and $replaceCommit -ne $env:GITHUB_SHA
if ($replaceCommit -and $release -and $release.target_commitish -ne $replaceCommit -and $release.target_commitish -ne $env:GITHUB_SHA) { throw 'Release replacement lease no longer matches' }
if ($release -and -not $release.draft -and -not $repairRelease) {
    Write-Output "Version $tag is already published: $($release.html_url)"
    return
}
if ($release -and $release.draft -and $release.target_commitish -ne $env:GITHUB_SHA -and -not $repairRelease) { throw 'Existing draft belongs to another commit' }
$assetNames = @('scribe-seek-windows-x64.exe', 'scribe-seek-windows-x64.zip', 'SHA256SUMS.txt')
foreach ($assetName in $assetNames) {
    if (-not (Test-Path -LiteralPath (Join-Path 'release-assets' $assetName))) { throw "Missing release asset: $assetName" }
}
$notes = @'
First public Windows x64 release of Scribe Seek.

- Local audio and video transcription with Whisper; recordings stay on your computer.
- Searchable transcripts, timestamp playback, waveform navigation, and TXT/SRT/VTT/JSON export.
- English interface by default, with Russian, French, German, Spanish and Italian in Settings.
- In-app GitHub update checks, SHA-256 verification, restart and rollback backup.
- Storage paths and buttons to open the model and data folders.
- Fixed clipped compact-button borders and sidebar layout.

Download the standalone EXE, or the ZIP including documentation and license notices. Windows 10/11 x64 and an AVX2/FMA/F16C-capable CPU are required. No Python, FFmpeg or installer is needed. Download a Whisper model once inside the app; model data is stored separately.

Preview builds reporting 0.1.0 can install this version from Settings → Updates. The standalone EXE asset and its GitHub SHA-256 digest are used by the updater.
'@
if ($repairRelease) {
    # An explicit commit trailer authorizes replacing exactly one known release
    # commit. Hide its assets until the replacement has passed verification.
    $draftBody = @{ draft = $true } | ConvertTo-Json
    Invoke-RestMethod -Method Patch -Uri "$releasesUrl/$($release.id)" -Headers $headers -Body $draftBody -ContentType 'application/json' | Out-Null
    $tagBody = @{ sha = $env:GITHUB_SHA; force = $true } | ConvertTo-Json
    Invoke-RestMethod -Method Patch -Uri "https://api.github.com/repos/$env:GITHUB_REPOSITORY/git/refs/tags/$tag" -Headers $headers -Body $tagBody -ContentType 'application/json' | Out-Null
    foreach ($oldAsset in @($release.assets | Where-Object { $_.name -in $assetNames })) {
        Invoke-RestMethod -Method Delete -Uri "$releasesUrl/assets/$($oldAsset.id)" -Headers $headers | Out-Null
    }
    $repairBody = @{ target_commitish = $env:GITHUB_SHA; body = $notes } | ConvertTo-Json
    $release = Invoke-RestMethod -Method Patch -Uri "$releasesUrl/$($release.id)" -Headers $headers -Body $repairBody -ContentType 'application/json'
}
if (-not $release) {
    $body = @{ tag_name = $tag; target_commitish = $env:GITHUB_SHA; name = "Scribe Seek $tag"; body = $notes; draft = $true; prerelease = $false } | ConvertTo-Json
    $release = Invoke-RestMethod -Method Post -Uri $releasesUrl -Headers $headers -Body $body -ContentType 'application/json'
}
foreach ($assetName in $assetNames) {
    $assetPath = Join-Path 'release-assets' $assetName
    $expectedDigest = 'sha256:' + (Get-FileHash -LiteralPath $assetPath -Algorithm SHA256).Hash.ToLower()
    $asset = $release.assets | Where-Object name -eq $assetName
    if (-not $asset) {
        $encodedName = [Uri]::EscapeDataString($assetName)
        $upload = "https://uploads.github.com/repos/$env:GITHUB_REPOSITORY/releases/$($release.id)/assets?name=$encodedName"
        $asset = Invoke-RestMethod -Method Post -Uri $upload -Headers $headers -InFile $assetPath -ContentType 'application/octet-stream'
    }
    if ($asset.digest -ne $expectedDigest) { throw "GitHub asset digest did not match: $assetName" }
}
$publishBody = @{ draft = $false } | ConvertTo-Json
Invoke-RestMethod -Method Patch -Uri "https://api.github.com/repos/$env:GITHUB_REPOSITORY/releases/$($release.id)" -Headers $headers -Body $publishBody -ContentType 'application/json' | Out-Null
Write-Output $release.html_url
