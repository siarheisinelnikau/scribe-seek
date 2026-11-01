# Runs after a successful Windows build. A new Cargo version on main creates
# its version tag and release; subsequent builds leave published versions alone.
param([switch]$RepairTagOnly)
$ErrorActionPreference = 'Stop'
$isVersionTag = $env:GITHUB_REF_TYPE -eq 'tag'
$isMainPush = $env:GITHUB_EVENT_NAME -eq 'push' -and $env:GITHUB_REF -eq 'refs/heads/main'
if (-not $env:GITHUB_TOKEN -or -not $env:GITHUB_REPOSITORY -or -not ($isVersionTag -or $isMainPush)) { throw 'A GitHub Actions main-push or version-tag run is required' }
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo version' }
$version = ($metadata.packages | Where-Object name -eq 'scribe-seek').version
if (-not $version) { throw 'Missing application version' }
$tag = "v$version"
if ($isMainPush -and $env:RELEASE_COMMIT_MESSAGE.StartsWith('Release localization v')) {
    $subject = ($env:RELEASE_COMMIT_MESSAGE -split "`n")[0].TrimEnd("`r")
    if ($subject -cne "Release localization $tag") { throw 'The final release subject must match the Cargo version.' }
}
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
if (-not $release) {
    # GitHub can detach the tag when a published release becomes a draft.
    $listedReleases = Invoke-RestMethod -Uri "$($releasesUrl)?per_page=100" -Headers $headers
    $detached = @($listedReleases | Where-Object { $_.name -eq "Scribe Seek $tag" -and $_.tag_name -like 'untagged-*' })
    if ($detached.Count -gt 1) { throw 'Multiple detached releases match this version' }
    if ($detached.Count -eq 1) { $release = $detached[0] }
}
$assetNames = @("ScribeSeek-$tag.exe", "ScribeSeek-$tag.zip", 'SHA256SUMS.txt', 'scribe-seek-windows-x64.exe')
if ($RepairTagOnly) {
    $repairTrailer = '(?m)^Repair-Release-Tag: ' + [regex]::Escape($tag) + '\r?$'
    if (-not $isMainPush -or $env:RELEASE_COMMIT_MESSAGE -notmatch $repairTrailer) { throw 'An explicit main-commit tag-repair trailer is required' }
    if (-not $release -or $release.draft -or $release.prerelease) { throw 'A complete published stable release is required for tag repair' }
    $tagRef = Invoke-RestMethod -Uri "https://api.github.com/repos/$env:GITHUB_REPOSITORY/git/ref/tags/$tag" -Headers $headers
    if ($tagRef.object.type -ne 'commit' -or $tagRef.object.sha -ne $release.target_commitish) { throw 'Release commit does not match the existing version tag' }
    foreach ($assetName in $assetNames) {
        $assets = @($release.assets | Where-Object name -eq $assetName)
        if ($assets.Count -ne 1 -or $assets[0].digest -notmatch '^sha256:[0-9a-f]{64}$' -or $assets[0].size -le 0) { throw "Missing verified release asset: $assetName" }
    }
    $repairBody = @{ tag_name = $tag } | ConvertTo-Json
    $published = Invoke-RestMethod -Method Patch -Uri "$releasesUrl/$($release.id)" -Headers $headers -Body $repairBody -ContentType 'application/json'
    if ($published.tag_name -ne $tag -or $published.draft) { throw 'GitHub did not restore the version tag' }
    Write-Output $published.html_url
    return
}
$replaceCommit = $null
if ($env:RELEASE_COMMIT_MESSAGE -match '(?m)^Replace-Release-Commit: ([0-9a-f]{40})\r?$') { $replaceCommit = $Matches[1] }
$repairRelease = $release -and $replaceCommit -and $release.target_commitish -eq $replaceCommit -and $replaceCommit -ne $env:GITHUB_SHA
if ($replaceCommit -and $release -and $release.target_commitish -ne $replaceCommit -and $release.target_commitish -ne $env:GITHUB_SHA) { throw 'Release replacement lease no longer matches' }
if ($release -and -not $release.draft -and -not $repairRelease) {
    if ($release.tag_name -ne $tag) { throw 'Published release has a detached tag; run explicit tag repair' }
    Write-Output "Version $tag is already published: $($release.html_url)"
    return
}
if ($release -and $release.draft -and $release.target_commitish -ne $env:GITHUB_SHA -and -not $repairRelease) { throw 'Existing draft belongs to another commit' }
foreach ($assetName in $assetNames) {
    if (-not (Test-Path -LiteralPath (Join-Path 'release-assets' $assetName))) { throw "Missing release asset: $assetName" }
}
$notesPath = Join-Path $PSScriptRoot "../releases/$tag.md"
if (-not (Test-Path -LiteralPath $notesPath)) { throw "Missing reviewed release notes: releases/$tag.md" }
$notes = Get-Content -LiteralPath $notesPath -Raw -Encoding utf8
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
$publishBody = @{ tag_name = $tag; draft = $false; prerelease = $false; make_latest = 'true' } | ConvertTo-Json
$published = Invoke-RestMethod -Method Patch -Uri "$releasesUrl/$($release.id)" -Headers $headers -Body $publishBody -ContentType 'application/json'
if ($published.tag_name -ne $tag -or $published.draft) { throw 'GitHub did not publish the requested version tag' }
Write-Output $published.html_url
