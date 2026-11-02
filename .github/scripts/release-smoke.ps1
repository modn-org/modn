# Smoke-tests a portable modn.exe away from any repository or Rust toolchain.
# Usage: pwsh -File release-smoke.ps1 -Binary <modn.exe> [-Checksum <modn.exe.sha256>]
[CmdletBinding()]
param(
  [Parameter(Mandatory)] [string] $Binary,
  [string] $Checksum
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-That([bool] $Condition, [string] $Message) {
  if (-not $Condition) { throw "smoke test failed: $Message" }
  Write-Host "ok: $Message"
}

$source = (Resolve-Path -LiteralPath $Binary).Path

if ($Checksum) {
  $expected = ((Get-Content -LiteralPath $Checksum -Raw).Trim() -split '\s+')[0]
  $actual = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
  Assert-That ($expected -ieq $actual) "SHA-256 matches $([IO.Path]::GetFileName($Checksum))"
}

$binaryText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($source))
$runtimeImports = [regex]::Matches($binaryText,'(?i)(vcruntime\d+(_\d+)?|msvcp\d+|api-ms-win-crt-[a-z-]+-l1-1-0)\.dll') |
  ForEach-Object Value | Sort-Object -Unique
Assert-That (-not $runtimeImports) "no Visual C++ runtime DLL imports ($($runtimeImports -join ', '))"

$env:Path = ($env:Path -split ';' | Where-Object {
    $_ -and -not (Test-Path -LiteralPath (Join-Path $_ 'cargo.exe')) -and
      -not (Test-Path -LiteralPath (Join-Path $_ 'rustc.exe'))
  }) -join ';'
Assert-That (-not (Get-Command cargo, rustc -ErrorAction SilentlyContinue)) 'cargo and rustc are not on PATH'

$work = Join-Path ([IO.Path]::GetTempPath()) "modn-smoke-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $work | Out-Null
try {
  Push-Location -LiteralPath $work
  Copy-Item -LiteralPath $source -Destination 'modn.exe'
  $insideRepository = (Get-Command git -ErrorAction SilentlyContinue) -and
    ($null -ne (git rev-parse --show-toplevel 2>$null))
  Assert-That (-not $insideRepository) "work directory $work is outside any Git repository"

  $game = Join-Path $work 'Cyberpunk 2077'
  New-Item -ItemType Directory -Path (Join-Path $game 'bin\x64') | Out-Null
  Set-Content -LiteralPath (Join-Path $game 'bin\x64\Cyberpunk2077.exe') -Value 'fixture'
  $mod = Join-Path $work 'mod\archive\pc\mod'
  New-Item -ItemType Directory -Path $mod | Out-Null
  Set-Content -LiteralPath (Join-Path $mod 'smoke.archive') -Value 'fixture'
  Compress-Archive -Path (Join-Path $work 'mod\archive') -DestinationPath 'smoke.zip'
  $gameBefore = Get-ChildItem -LiteralPath $game -Recurse -Force | ForEach-Object FullName

  function Invoke-Modn([string[]] $Arguments) {
    $output = & .\modn.exe --json --state-dir state --game cyberpunk-2077 @Arguments
    if ($LASTEXITCODE -ne 0) { throw "modn $($Arguments -join ' ') exited with $LASTEXITCODE" }
    return ($output -join "`n") | ConvertFrom-Json
  }

  $search = Invoke-Modn @('search')
  $red4ext = @($search | Where-Object { $_.bucket -eq 'main' -and $_.id -eq 'red4ext' })
  Assert-That ($red4ext.Count -eq 1) "search lists red4ext from the bundled main bucket ($(@($search).Count) mods)"

  $catalog = Invoke-Modn @('install', 'red4ext', '--dry-run', '--path', $game)
  Assert-That ($catalog.package -eq 'main/cyberpunk-2077/red4ext' -and $null -eq $catalog.plan) `
    'install red4ext --dry-run resolves the catalog mod without downloading it'

  $archive = Invoke-Modn @('install', 'smoke.zip', '--dry-run', '--path', $game)
  $operations = @($archive.operations)
  Assert-That ($archive.status -eq 'reviewable' -and $operations.Count -eq 1 -and
    $operations[0].kind -eq 'legacy-archive' -and $operations[0].destination -eq 'archive/pc/mod/smoke.archive') `
    'install smoke.zip --dry-run plans one legacy archive through the bundled Lua policy'

  $doctor = Invoke-Modn @('doctor', '--path', $game)
  Assert-That ($doctor.scan.game_id -eq 'cyberpunk-2077' -and
    @($doctor.diagnostics | Where-Object code -eq 'REDMOD_NOT_FOUND').Count -eq 1) `
    'doctor reports the bundled policy diagnostic REDMOD_NOT_FOUND'

  $gameAfter = Get-ChildItem -LiteralPath $game -Recurse -Force | ForEach-Object FullName
  Assert-That (-not (Compare-Object @($gameBefore) @($gameAfter))) 'the fake game directory is unchanged'
} finally {
  Pop-Location
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host 'release smoke test passed'
