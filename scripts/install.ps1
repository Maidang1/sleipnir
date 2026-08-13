# install.ps1 — Download the latest Sleipnir Windows zip into %LOCALAPPDATA%\Sleipnir
# and verify it against the published .sha256 sidecar.
#
# Usage:
#   irm https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.ps1 | iex
#   $env:PREFIX = "$env:USERPROFILE\Apps\Sleipnir"; .\scripts\install.ps1
#
# Environment:
#   PREFIX           Install directory (default: %LOCALAPPDATA%\Sleipnir)
#   SLEIPNIR_NO_OPEN Set to 1 to skip launching the app after install
#   SLEIPNIR_REPO    GitHub owner/repo (default: Maidang1/sleipnir)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = if ($env:SLEIPNIR_REPO) { $env:SLEIPNIR_REPO } else { 'Maidang1/sleipnir' }
$Prefix = if ($env:PREFIX) { $env:PREFIX } else { Join-Path $env:LOCALAPPDATA 'Sleipnir' }
$AppName = 'Sleipnir'
$UserAgent = 'sleipnir-install'

function Get-LatestTag {
    $request = [System.Net.HttpWebRequest]::Create("https://github.com/$Repo/releases/latest")
    $request.AllowAutoRedirect = $false
    $request.UserAgent = $UserAgent
    $request.Method = 'HEAD'
    try {
        $response = $request.GetResponse()
    } catch [System.Net.WebException] {
        $response = $_.Exception.Response
        if (-not $response) { throw }
    }
    try {
        $location = $response.Headers['Location']
        if (-not $location) {
            throw "could not resolve latest tag (no Location header)"
        }
        ($location -split '/')[-1]
    } finally {
        $response.Close()
    }
}

Write-Host '=== Sleipnir install ==='
Write-Host "  repo:   $Repo"
Write-Host "  dest:   $Prefix"

Write-Host '  fetching latest release…'
$Tag = Get-LatestTag
$Version = $Tag.TrimStart('v')
if (-not $Version) {
    throw "could not parse version from tag '$Tag'"
}

$ZipName = "$AppName-$Version-windows-x64.zip"
$ZipUrl = "https://github.com/$Repo/releases/download/$Tag/$ZipName"
$ShaUrl = "$ZipUrl.sha256"
Write-Host "  version: $Version ($Tag)"

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("sleipnir-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Work | Out-Null
try {
    $Zip = Join-Path $Work $ZipName
    Write-Host "  downloading $ZipName…"
    Invoke-WebRequest -Uri $ZipUrl -OutFile $Zip -UseBasicParsing -UserAgent $UserAgent

    $ShaFile = "$Zip.sha256"
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaFile -UseBasicParsing -UserAgent $UserAgent
    $Want = ((Get-Content -Raw $ShaFile) -replace '\s', '').ToLowerInvariant()
    $Got = (Get-FileHash -Algorithm SHA256 -Path $Zip).Hash.ToLowerInvariant()
    if ($Want -ne $Got) {
        throw "SHA-256 mismatch`n  expected: $Want`n  got:      $Got"
    }
    Write-Host "  sha256: $Got  ok"

    Write-Host '  unpacking…'
    Expand-Archive -Path $Zip -DestinationPath $Work -Force
    $Exe = Join-Path $Work 'sleipnir.exe'
    if (-not (Test-Path $Exe)) {
        throw "archive did not contain sleipnir.exe"
    }

    New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
    $Dest = Join-Path $Prefix 'sleipnir.exe'
    Copy-Item -Path $Exe -Destination $Dest -Force
    Write-Host "  installed: $Dest"

    if ($env:SLEIPNIR_NO_OPEN -ne '1') {
        Start-Process -FilePath $Dest
    }
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}

Write-Host '=== done ==='
