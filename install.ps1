# Download, verify, and install vork on Windows x64 without gh or admin rights.
#   & ([scriptblock]::Create((irm https://github.com/voyager-so/vork/releases/latest/download/install.ps1)))
# Environment: VERSION, PREFIX (default %LOCALAPPDATA%\Programs\vork), VORK_REPO, VORK_DOWNLOAD_BASE;
# VORK_SKIP_SETUP=1 skips `vork setup` (GPU-free CI); VORK_NO_MODIFY_PATH=1 leaves the user PATH alone.
# A custom PREFIX never edits it.
param(
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = if ($env:VORK_REPO) { $env:VORK_REPO } else { 'voyager-so/vork' }
# Default version is generated from Cargo.toml by scripts/sync-version.
$version = if ($env:VERSION) { $env:VERSION } else { '0.1.0' }
$prefix = if ($env:PREFIX) { $env:PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs\vork' }
if ($version -notmatch '^[A-Za-z0-9._-]+$') { throw 'Invalid VERSION' }
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') { throw "Supported platforms: Windows x64, Apple Silicon macOS and Linux x64; found $arch" }
$archive = "vork-$version-windows-x64.zip"
$base = if ($env:VORK_DOWNLOAD_BASE) { $env:VORK_DOWNLOAD_BASE.TrimEnd('/') } else { "https://github.com/$repo/releases/download/v$version" }
$bin = $prefix
if ($DryRun) {
    "Archive: $base/$archive"
    "Checksums: $base/SHA256SUMS"
    "Binary: $bin\vork.exe"
    return
}
function Get-Release([string]$name, [string]$target) {
    $url = "$base/$name"
    try {
        if ($url -match '^file:') { Copy-Item -LiteralPath ([Uri]$url).LocalPath -Destination $target }
        else { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $target }
    } catch {
        throw "Could not download $url ($($_.Exception.Message)) Nothing was installed. Check your connection and that vork $version has this file: https://github.com/$repo/releases"
    }
}
# Add a folder to the user PATH, keeping unexpanded entries such as %USERPROFILE%.
function Add-UserPath([string]$folder) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
    try {
        $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if (($raw -split ';') -contains $folder) { return }
        $key.SetValue('Path', ($folder + $(if ($raw) { ";$raw" })), [Microsoft.Win32.RegistryValueKind]::ExpandString)
    } finally { $key.Close() }
    # Setting any user variable tells Explorer to reload the environment for new windows.
    [Environment]::SetEnvironmentVariable('VORK_INSTALL_NOTIFY', '1', 'User')
    [Environment]::SetEnvironmentVariable('VORK_INSTALL_NOTIFY', $null, 'User')
}
$work = Join-Path ([IO.Path]::GetTempPath()) ("vork-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$stage = "$prefix.new-$PID"
$retired = "$prefix.old-$PID"
try {
    $zip = Join-Path $work $archive
    Get-Release $archive $zip
    Get-Release 'SHA256SUMS' (Join-Path $work 'SHA256SUMS')
    $rows = @(Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object {
            $f = -split $_; $f.Count -eq 2 -and ($f[1] -eq $archive -or $f[1] -eq "*$archive") })
    if ($rows.Count -ne 1) { throw "Missing or ambiguous checksum for $archive; nothing was installed." }
    $expected = (-split $rows[0])[0]
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
    if ($actual -ne $expected) { throw "Checksum mismatch for ${archive}: the download is damaged or altered. Nothing was installed; run this again." }
    "${archive}: OK"
    # Refuse traversal paths before extracting a verified archive.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $z = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($e in $z.Entries) {
            $n = $e.FullName -replace '\\', '/'
            if ($n -match '^/|^[A-Za-z]:|(^|/)\.\.(/|$)') { throw 'Unsafe archive path' }
        }
    } finally { $z.Dispose() }
    # Build the new tree beside the old one and swap it in whole: files a release dropped
    # do not survive an upgrade, and a failed install leaves the old one working.
    $running = @(Get-Process -Name vork -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $bin 'vork.exe') })
    if ($running.Count) { throw "vork is still running from $prefix (process $($running[0].Id)). Let it finish or close it, then run this again. Your existing install was not changed." }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $prefix) | Out-Null
    Expand-Archive -LiteralPath $zip -DestinationPath $stage
    if (Test-Path -LiteralPath $prefix) {
        try { Move-Item -LiteralPath $prefix -Destination $retired }
        catch { throw "Could not replace $prefix ($($_.Exception.Message)) Close anything using that folder and run this again. Your existing install was not changed." }
    }
    Move-Item -LiteralPath $stage -Destination $prefix
    $exe = Join-Path $bin 'vork.exe'
    & $exe --version
    if ($LASTEXITCODE -ne 0) { throw 'vork --version failed' }
    # Let a new window find vork. Only the default location touches the user PATH.
    $pathNote = $null
    if ($env:PREFIX -or $env:VORK_NO_MODIFY_PATH -eq '1') {
        if (-not (($env:Path -split ';') -contains $bin)) {
            $pathNote = "Add it to PATH with:`n  [Environment]::SetEnvironmentVariable('Path', `"$bin;`" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
        }
    } else {
        Add-UserPath $bin
        if (-not (($env:Path -split ';') -contains $bin)) { $env:Path = "$bin;$env:Path" }
        $pathNote = "New PowerShell windows find vork. In a window that is already open, run:`n  `$env:Path = `"$bin;`$env:Path`""
    }
    if ($env:VORK_SKIP_SETUP -eq '1') {
        'Skipping vork setup (VORK_SKIP_SETUP=1); run vork setup before rendering.'
    } else {
        'Preparing shaders for this GPU (up to about 3 minutes)...'
        & $exe setup
        if ($LASTEXITCODE -ne 0) {
            if ($pathNote) { $pathNote }
            throw "vork is installed at $exe, but setup failed. Follow the report above, then run: $exe setup"
        }
    }
    "Installed to $exe."
    if ($pathNote) { $pathNote }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $retired -Recurse -Force -ErrorAction SilentlyContinue
}
