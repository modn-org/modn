$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ("modn-install-smoke-" + [guid]::NewGuid().ToString('N'))
$previousData = $env:LOCALAPPDATA
$previousSource = $env:MODN_MAIN_BUCKET_SOURCE
$checks = 0
function Assert-That([bool] $Condition, [string] $Message) {
  if (-not $Condition) { throw "smoke test failed: $Message" }
  $script:checks++
  Write-Host "ok: $Message"
}
try {
  New-Item -ItemType Directory -Path $work | Out-Null
  $assets = Join-Path $work 'assets'
  $install = Join-Path $work 'bin'
  New-Item -ItemType Directory -Path $assets | Out-Null
  cargo build --release --locked -p modn-cli
  if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
  Copy-Item (Join-Path $root 'target/release/modn.exe') (Join-Path $assets 'modn.exe')
  $hash = (Get-FileHash (Join-Path $assets 'modn.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText((Join-Path $assets 'modn.exe.sha256'), "$hash *modn.exe`n")
  python (Join-Path $root '.github/scripts/pack-main-bucket.py') (Join-Path $root 'buckets/main') (Join-Path $assets 'main-bucket.zip')
  if ($LASTEXITCODE -ne 0) { throw 'bucket packing failed' }
  $env:LOCALAPPDATA = Join-Path $work 'data'
  $env:MODN_MAIN_BUCKET_SOURCE = $null
  $script = Join-Path $root 'install.ps1'
  & $script -ReleaseBase $assets -InstallDir $install -NoModifyPath
  $exe = Join-Path $install 'modn.exe'
  Assert-That (Test-Path -LiteralPath $exe) 'installed executable exists'
  $search = & $exe --json --game cyberpunk-2077 search | ConvertFrom-Json
  Assert-That ($LASTEXITCODE -eq 0 -and @($search | Where-Object { $_.id -eq 'red4ext' -and $_.bucket -eq 'main' }).Count -eq 1) 'search lists main/red4ext'
  & $script -ReleaseBase $assets -InstallDir $install -NoModifyPath
  Assert-That ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $hash) 'reinstall preserves executable'
  [IO.File]::WriteAllText((Join-Path $assets 'modn.exe.sha256'), ('0' * 64))
  $failed = $false
  try { & $script -ReleaseBase $assets -InstallDir $install -NoModifyPath } catch { $failed = $true }
  Assert-That $failed 'tampered checksum fails'
  Assert-That ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $hash) 'tampered checksum leaves executable unchanged'
  Write-Host "Passed $checks assertions."
} finally {
  $env:LOCALAPPDATA = $previousData
  $env:MODN_MAIN_BUCKET_SOURCE = $previousSource
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
