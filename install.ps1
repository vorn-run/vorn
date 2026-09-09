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

    # The `vorn` command, beside the app and inside the directory just added to
    # PATH. It runs the app's own Node, so nothing else has to be installed and
    # the native modules inside the bundle resolve.
    # Joined with CRLF rather than written as a here-string: this file may be
    # fetched with LF endings, and cmd.exe is unreliable about a multi-line
    # block that arrives that way.
    $ShimLines = @(
        '@echo off',
        'setlocal',
        'set "APPDIR=%~dp0"',
        'if "%~1"=="" (',
        '  start "" "%APPDIR%Vorn.exe"',
        '  exit /b',
        ')',
        'set "ELECTRON_RUN_AS_NODE=1"',
        'set "VORN_NATIVE_MODULES_PATH=%APPDIR%resources\app.asar.unpacked\node_modules"',
        'set "NODE_PATH=%APPDIR%resources\app.asar\node_modules;%VORN_NATIVE_MODULES_PATH%"',
        '"%APPDIR%Vorn.exe" "%APPDIR%resources\server\cli.cjs" %*'
    )
    $Shim = ($ShimLines -join "`r`n") + "`r`n"
    Set-Content -Path (Join-Path $InstallDir "vorn.cmd") -Value $Shim -Encoding ASCII -NoNewline

    Write-Host ""
    Write-Host "$AppName $Version installed to $InstallDir"
    Write-Host "Launch from Start Menu, desktop shortcut, or run '$AppName' in a new terminal."
    Write-Host "The vorn command is available in a new terminal: vorn --help"
} finally {
    Write-Host "Cleaning up..."
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}

Write-Host "Done!"
