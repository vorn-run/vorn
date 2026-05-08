$ErrorActionPreference = "Stop"

$Repo = "vorn-run/vorn"
$AppName = "Vorn"

function Get-LatestVersion {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    return $release.tag_name
}

$Version = if ($env:VORN_VERSION) { $env:VORN_VERSION } else { Get-LatestVersion }

if (-not $Version) {
    Write-Error "Could not determine latest version. Set VORN_VERSION=vX.Y.Z to install a specific version."
    exit 1
}

$VersionNum = $Version.TrimStart("v")

Write-Host "Installing $AppName $Version..."

$Artifact = "$AppName-Setup-$VersionNum.exe"
$Url = "https://github.com/$Repo/releases/download/$Version/$Artifact"
$TempDir = Join-Path $env:TEMP "vorn-install"
$InstallerPath = Join-Path $TempDir $Artifact

New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

Write-Host "Downloading $Artifact..."
Invoke-WebRequest -Uri $Url -OutFile $InstallerPath -UseBasicParsing

try {
    Write-Host "Running installer..."
    $process = Start-Process -FilePath $InstallerPath -ArgumentList "/S" -Wait -PassThru

    if ($process.ExitCode -ne 0) {
        Write-Error "Installer exited with code $($process.ExitCode)."
        exit 1
    }

    # Verify installation at the default per-user location
    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
    $ExePath = Join-Path $InstallDir "$AppName.exe"

    if (-not (Test-Path $ExePath)) {
        Write-Error "Installation could not be verified — $ExePath not found."
        Write-Host "Try running the installer manually: $InstallerPath"
        exit 1
    }

    # Add install directory to user PATH if not already present
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathEntries = @()
    if ($UserPath) {
        $pathEntries = $UserPath -split ';' | ForEach-Object { $_.Trim() } | Where-Object { $_ }
    }
    if (-not ($pathEntries -contains $InstallDir)) {
        $pathEntries += $InstallDir
        $newPath = ($pathEntries -join ';')
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Host "Added $InstallDir to your PATH."
    }

    Write-Host ""
    Write-Host "$AppName $Version installed to $InstallDir"
    Write-Host "Launch from Start Menu, desktop shortcut, or run '$AppName' in a new terminal."
} finally {
    Write-Host "Cleaning up..."
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}

Write-Host "Done!"
