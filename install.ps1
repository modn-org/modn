[CmdletBinding()]
param(
  [string] $InstallDir = (Join-Path $env:LOCALAPPDATA 'modn\bin'),
  [string] $ReleaseBase = 'https://github.com/modn-org/modn/releases/latest/download',
  [switch] $NoModifyPath
)

$ErrorActionPreference = 'Stop'
$local = Test-Path -LiteralPath $ReleaseBase -PathType Container
if (-not $local) {
  $uri = $null
  if (-not [Uri]::TryCreate($ReleaseBase, [UriKind]::Absolute, [ref] $uri) -or $uri.Scheme -ne 'https') {
    throw 'ReleaseBase must be an HTTPS URL or an existing local directory.'
  }
}
$work = Join-Path ([IO.Path]::GetTempPath()) ("modn-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
  foreach ($name in @('modn.exe', 'modn.exe.sha256')) {
    $destination = Join-Path $work $name
    if ($local) { Copy-Item -LiteralPath (Join-Path $ReleaseBase $name) -Destination $destination }
    else { Invoke-WebRequest -Uri ($ReleaseBase.TrimEnd('/') + '/' + $name) -OutFile $destination -UseBasicParsing }
  }
  $checksum = Get-Content -LiteralPath (Join-Path $work 'modn.exe.sha256') -Raw
  if ($checksum -cnotmatch '^([0-9a-f]{64})(?:\s+\S+)?\s*$') { throw 'Invalid modn.exe.sha256.' }
  $expected = $Matches[1]
  $binary = Join-Path $work 'modn.exe'
  if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) {
    throw 'modn.exe SHA-256 mismatch; installation was not changed.'
  }
  $InstallDir = [IO.Path]::GetFullPath($InstallDir)
  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  $target = Join-Path $InstallDir 'modn.exe'
  $staged = Join-Path $InstallDir ('.modn-' + [guid]::NewGuid().ToString('N') + '.tmp')
  try {
    Copy-Item -LiteralPath $binary -Destination $staged
    if (Test-Path -LiteralPath $target) {
      $backup = Join-Path $work 'previous.exe'
      [IO.File]::Replace($staged, $target, $backup)
    }
    else { [IO.File]::Move($staged, $target) }
  } finally { if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Force } }
  Write-Host "Installed modn.exe to $target"
  if (-not $NoModifyPath) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($userPath -split ';' | Where-Object { $_ })
    $normalized = $InstallDir.TrimEnd('\', '/')
    if (-not @($entries | Where-Object { $_.Trim().TrimEnd('\', '/') -ieq $normalized }).Count) {
      [Environment]::SetEnvironmentVariable('Path', (($entries + $InstallDir) -join ';'), 'User')
      Write-Host "Added $InstallDir to the user PATH."
    }
  }
  $previousSource = $env:MODN_MAIN_BUCKET_SOURCE
  try {
    if ($local) { $env:MODN_MAIN_BUCKET_SOURCE = (Resolve-Path -LiteralPath (Join-Path $ReleaseBase 'main-bucket.zip')).Path }
    & $target bucket update
    if ($LASTEXITCODE -ne 0) { throw "modn bucket update failed with exit code $LASTEXITCODE." }
  } finally { $env:MODN_MAIN_BUCKET_SOURCE = $previousSource }
  Write-Host 'Run modn search to discover packages. Open a new terminal if PATH changed.'
} finally { Remove-Item -LiteralPath $work -Recurse -Force }
