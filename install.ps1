# Install Marrow's prebuilt binaries on Windows — no Rust toolchain required.
#
#   irm https://raw.githubusercontent.com/aryawidjaja/marrow/main/install.ps1 | iex
#
# Installs marrow, marrow-mcp, marrow-serve and marrow-server into
# %LOCALAPPDATA%\Programs\marrow and puts that on your PATH. No admin rights needed.
# Override the destination with $env:MARROW_BIN_DIR, or the version with $env:MARROW_VERSION.

$ErrorActionPreference = 'Stop'
$repo = 'aryawidjaja/marrow'
$binDir = if ($env:MARROW_BIN_DIR) { $env:MARROW_BIN_DIR } else { "$env:LOCALAPPDATA\Programs\marrow" }

$arch = (Get-CimInstance Win32_Processor).Architecture
if ($arch -ne 9) {
  # 9 = x64. Arm64 Windows has no prebuilt binary yet; building from source still works.
  Write-Host "No prebuilt binary for this processor architecture." -ForegroundColor Yellow
  Write-Host "Install with Rust instead:"
  Write-Host "  cargo install --git https://github.com/$repo marrow-cli marrow-mcp marrow-web marrow-server"
  exit 1
}
$target = 'x86_64-pc-windows-msvc'

$tag = $env:MARROW_VERSION
if (-not $tag) {
  try {
    $tag = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
  } catch {
    throw "Could not reach the GitHub releases API: $($_.Exception.Message)"
  }
}
if (-not $tag) { throw "No published release found yet." }

$asset = "marrow-$tag-$target.tar.gz"
$url = "https://github.com/$repo/releases/download/$tag/$asset"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("marrow-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

try {
  Write-Host "Downloading $url"
  Invoke-WebRequest -Uri $url -OutFile "$tmp\$asset" -UseBasicParsing

  # Verify the checksum, the same way the shell installer does. A release without one is only
  # installed when the caller opts in, so a tampered or truncated download cannot run silently.
  try {
    Invoke-WebRequest -Uri "$url.sha256" -OutFile "$tmp\$asset.sha256" -UseBasicParsing
    $expected = ((Get-Content "$tmp\$asset.sha256" -Raw) -split '\s+')[0].Trim().ToLower()
    $actual = (Get-FileHash "$tmp\$asset" -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) {
      throw "Checksum verification failed for $asset."
    }
  } catch [System.Net.WebException] {
    if ($env:MARROW_ALLOW_UNVERIFIED -ne '1') {
      throw "This release has no checksum, so the installer will not run it. Choose a newer release, or set `$env:MARROW_ALLOW_UNVERIFIED='1' for a legacy release."
    }
    Write-Host "Warning: installing a legacy release without verification." -ForegroundColor Yellow
  }

  # tar ships with Windows 10 1803 and later, so no extra tooling is needed.
  tar -xzf "$tmp\$asset" -C $tmp
  if ($LASTEXITCODE -ne 0) { throw "Could not extract $asset." }

  New-Item -ItemType Directory -Path $binDir -Force | Out-Null
  foreach ($bin in 'marrow', 'marrow-mcp', 'marrow-serve', 'marrow-server') {
    $src = Get-ChildItem -Path $tmp -Recurse -Filter "$bin.exe" | Select-Object -First 1
    if ($src) { Copy-Item $src.FullName (Join-Path $binDir "$bin.exe") -Force }
  }
  # The semantic build links ONNX Runtime; ship whatever native libraries came with it.
  Get-ChildItem -Path $tmp -Recurse -Filter '*.dll' | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $binDir $_.Name) -Force
  }
} finally {
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Installed marrow, marrow-mcp, marrow-serve and marrow-server to $binDir"

# Put it on PATH for future shells, and for this one, so the next line works immediately.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$binDir*") {
  [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
  Write-Host "Added $binDir to your PATH (open a new terminal for other apps to see it)."
}
$env:Path = "$env:Path;$binDir"

$missing = @()
foreach ($dep in 'bash', 'jq') {
  if (-not (Get-Command $dep -ErrorAction SilentlyContinue)) { $missing += $dep }
}
if ($missing.Count -gt 0) {
  Write-Host ""
  Write-Host "Marrow's hooks are shell scripts and need $($missing -join ' and ')." -ForegroundColor Yellow
  Write-Host "Memory works without them, but warm starts, collision checks and activity capture stay off."
  Write-Host "  winget install Git.Git jqlang.jq"
}

Write-Host @"

Next steps:
  cd your-project
  marrow setup            # wire this project into Claude Code (add --global for every project)
  # then restart Claude Code

Onboarding an existing repo? Run 'marrow ingest' (or ask your agent to "seed marrow from this
repo's docs"). Capture a session anytime with /marrow-save.

Docs: https://github.com/$repo
"@
