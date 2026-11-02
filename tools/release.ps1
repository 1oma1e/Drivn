# Builds the Drivn setup for a release (ADR-0027, ADR-0028): drivn-installer.exe, then the NSIS
# setup signed with the update key (kept outside the repo), then latest.json for the updater.
# Output in dist\ : Drivn_<version>_x64-setup.exe, its .sig, latest.json. Upload the three to the
# GitHub release v<version>. Nothing here is run on a driver or a device.
# ASCII only on purpose: Windows PowerShell 5.1 reads BOM-less scripts as ANSI.
param(
    [string]$Notes = 'Nouvelle version de Drivn.',
    [string]$Key = 'F:\DrivnKeys\drivn-update.key'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$repo = '1oma1e/Drivn'
if (-not (Test-Path -LiteralPath $Key)) { throw "Private update key not found: $Key" }

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$version = (Get-Content -Raw (Join-Path $root 'app\src-tauri\tauri.conf.json') | ConvertFrom-Json).version

Push-Location $root
try {
    cargo build --release -p drivn-installer
    if ($LASTEXITCODE -ne 0) { throw 'drivn-installer build failed' }
    $bin = Join-Path $root 'app\src-tauri\bin'
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item (Join-Path $target 'release\drivn-installer.exe') (Join-Path $bin 'drivn-installer.exe') -Force

    $env:TAURI_SIGNING_PRIVATE_KEY = $Key
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
    Push-Location (Join-Path $root 'app\src-tauri')
    try {
        cargo tauri build --config tauri.bundle.conf.json
        if ($LASTEXITCODE -ne 0) { throw 'cargo tauri build failed' }
    } finally { Pop-Location }
} finally {
    Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Pop-Location
}

$setup = Get-ChildItem (Join-Path $target 'release\bundle\nsis') -Filter "*_${version}_x64-setup.exe" | Select-Object -First 1
if (-not $setup) { throw "setup of version $version not found" }
$sig = Get-Content -Raw "$($setup.FullName).sig"

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item $setup.FullName $dist -Force
Copy-Item "$($setup.FullName).sig" $dist -Force
$latest = [ordered]@{
    version   = $version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{
            signature = $sig.Trim()
            url       = "https://github.com/$repo/releases/download/v$version/$($setup.Name)"
        }
    }
}
[IO.File]::WriteAllText((Join-Path $dist 'latest.json'), ($latest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
Write-Output "release ${version}: $dist"
Get-ChildItem $dist | Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Output
