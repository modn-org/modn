[CmdletBinding(DefaultParameterSetName = 'Release')]
param(
  [Parameter(Mandatory, ParameterSetName = 'Release')] [string] $Tag,
  [Parameter(Mandatory, ParameterSetName = 'Local')] [string] $Binary,
  [Parameter(Mandatory, ParameterSetName = 'Local')] [string] $Checksum,
  [string] $Repository = 'modn-org/modn',
  [string] $VerifierBinaries
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$results = [Collections.Generic.List[object]]::new()
$work = Join-Path ([IO.Path]::GetTempPath()) "modn-m2-$([guid]::NewGuid().ToString('N'))"
$saved = @{}
$names = @('Path', 'LOCALAPPDATA', 'APPDATA', 'XDG_STATE_HOME', 'XDG_CACHE_HOME',
  'MODN_CACHE', 'MODN_MAIN_BUCKET_SOURCE', 'MODN_GAME_MODULES_DIR', 'TEMP', 'TMP',
  'AGENT_DESKTOP_HOME', 'PYTHONDONTWRITEBYTECODE', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM')
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }

function Snapshot([string] $Root) {
  if (-not $Root -or -not (Test-Path -LiteralPath $Root)) { return 'absent' }
  @('present'; Get-ChildItem -LiteralPath $Root -Recurse -Force | Sort-Object FullName | ForEach-Object {
    if ($_.PSIsContainer) { "D|$($_.FullName)" }
    else { "F|$($_.FullName)|$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)" }
  }) -join "`n"
}

function Require([bool] $Condition, [string] $Message) {
  if (-not $Condition) { throw $Message }
}

function Check([string] $Name, [scriptblock] $Action) {
  try {
    & $Action
    $results.Add([pscustomobject]@{ name = $Name; status = 'PASS'; error = $null })
    Write-Host "PASS $Name"
  } catch {
    $results.Add([pscustomobject]@{ name = $Name; status = 'FAIL'; error = "$_" })
    Write-Host "FAIL ${Name}: $_"
  }
}

function Invoke-GateModn([string[]] $Arguments, [switch] $Refuse) {
  $output = & $script:modn --json --state-dir (Join-Path $work 'state') --game cyberpunk-2077 @Arguments 2>&1 | Out-String
  $code = $LASTEXITCODE
  if ($Refuse) {
    Require ($code -ne 0) "Expected refusal: $($Arguments -join ' ')"
    return $output
  }
  Require ($code -eq 0) "modn $($Arguments -join ' ') exited ${code}: $output"
  $output | ConvertFrom-Json
}

function Write-File([string] $Path, [string] $Text) {
  New-Item -ItemType Directory -Path (Split-Path $Path) -Force | Out-Null
  [IO.File]::WriteAllText($Path, $Text)
}

function Zip([string] $Name, [hashtable] $Files) {
  $path = Join-Path $work "$Name.zip"
  $archive = [IO.Compression.ZipFile]::Open($path, 'Create')
  try {
    foreach ($name in $Files.Keys) {
      $writer = [IO.StreamWriter]::new($archive.CreateEntry($name).Open())
      try { $writer.Write($Files[$name]) } finally { $writer.Dispose() }
    }
  } finally { $archive.Dispose() }
  return $path
}

function Run-Python([string] $Driver, [string[]] $Arguments) {
  & python -B (Join-Path $repo $Driver) @Arguments
  Require ($LASTEXITCODE -eq 0) "$Driver failed with exit $LASTEXITCODE"
}

function Run-Desktop([string] $Driver, [string[]] $Arguments) {
  $output = @(& python -B (Join-Path $repo $Driver) @Arguments)
  Require ($LASTEXITCODE -eq 0) "$Driver failed with exit $LASTEXITCODE"
  $output | Write-Host
  $summary = $output[-1] | ConvertFrom-Json
  Require ($summary.status -eq 'passed' -and $summary.assertions -gt 0 -and
    $summary.command_nonzero -eq 0 -and $summary.app_exit -eq 0 -and
    $summary.fixture_removed) "$Driver did not complete its checks and cleanup"
}

function Root-Report([hashtable] $Snapshots) {
  foreach ($path in $Snapshots.Keys) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Snapshots[$path])
    [pscustomobject]@{ path = $path; entries = @($Snapshots[$path] -split "`n").Count
      sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant() }
  }
}

$roots = @{}
foreach ($path in @(
    $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'modn' }),
    $(if ($env:XDG_STATE_HOME) { Join-Path $env:XDG_STATE_HOME 'modn' }),
    $(if ($env:HOME) { Join-Path $env:HOME '.local/state/modn' }),
    $env:MODN_CACHE)) {
  if ($path) { $roots[$path] = Snapshot $path }
}
$localBinary = if ($Binary) { (Resolve-Path -LiteralPath $Binary).Path }
$localChecksum = if ($Checksum) { (Resolve-Path -LiteralPath $Checksum).Path }
if (-not $VerifierBinaries) {
  $VerifierBinaries = Join-Path $repo 'target/debug'
}
$VerifierBinaries = [IO.Path]::GetFullPath($VerifierBinaries)
New-Item -ItemType Directory -Path $work | Out-Null
@(Root-Report $roots) | ConvertTo-Json | Set-Content (Join-Path $work 'roots-before.json')
Write-Host "Evidence and disposable fixtures: $work"
try {
  foreach ($name in @('LOCALAPPDATA', 'APPDATA', 'XDG_STATE_HOME', 'XDG_CACHE_HOME',
      'MODN_CACHE', 'AGENT_DESKTOP_HOME', 'TEMP', 'TMP')) {
    $path = Join-Path $work $name
    if ($name -ne 'AGENT_DESKTOP_HOME') {
      New-Item -ItemType Directory -Path $path | Out-Null
    }
    [Environment]::SetEnvironmentVariable($name, $path)
  }
  $env:MODN_MAIN_BUCKET_SOURCE = $null
  $env:MODN_GAME_MODULES_DIR = $null
  $env:PYTHONDONTWRITEBYTECODE = '1'
  $env:GIT_CONFIG_GLOBAL = Join-Path $work 'gitconfig'
  $env:GIT_CONFIG_NOSYSTEM = '1'
  $env:Path = ($env:Path -split ';' | Where-Object {
    $_ -and -not (Test-Path -LiteralPath (Join-Path $_ 'cargo.exe')) -and
      -not (Test-Path -LiteralPath (Join-Path $_ 'rustc.exe'))
  }) -join ';'
  Push-Location $work
  Check 'No Rust toolchain on PATH' {
    Require (-not (Get-Command cargo, rustc -ErrorAction SilentlyContinue)) 'Rust is still on PATH'
  }
  Check 'Asset checksum' {
    $script:modn = Join-Path $work 'modn.exe'
    $checksumFile = Join-Path $work 'modn.exe.sha256'
    if ($localBinary) {
      Copy-Item -LiteralPath $localBinary -Destination $script:modn
      Copy-Item -LiteralPath $localChecksum -Destination $checksumFile
    } else {
      $base = "https://github.com/$Repository/releases/download/$Tag"
      foreach ($name in @('modn.exe', 'modn.exe.sha256')) {
        Invoke-WebRequest "$base/$name" -OutFile (Join-Path $work $name)
      }
    }
    $expected = ((Get-Content $checksumFile -Raw).Trim() -split '\s+')[0]
    Require ($expected -match '^[a-fA-F0-9]{64}$') 'Invalid SHA-256 sidecar'
    Require ((Get-FileHash $script:modn -Algorithm SHA256).Hash -ieq $expected) 'Checksum mismatch'
  }
  Require ($results[-1].status -eq 'PASS') 'Cannot run an unverified executable'
  Check 'Milestone 1 exit gate' {
    $gate = Join-Path $PSScriptRoot 'milestone1-gate.ps1'
    if ($localBinary) {
      & pwsh -NoProfile -File $gate -Binary $script:modn -Checksum (Join-Path $work 'modn.exe.sha256')
    } else {
      & pwsh -NoProfile -File $gate -Tag $Tag -Repository $Repository
    }
    Require ($LASTEXITCODE -eq 0) 'Milestone 1 gate failed'
  }
  Check 'Disposable main bucket' {
    & python -B (Join-Path $repo 'buckets/main/.github/scripts/pack.py') (Join-Path $repo 'buckets/main') (Join-Path $work 'main.zip')
    Require ($LASTEXITCODE -eq 0) 'Bucket packing failed'
    $env:MODN_MAIN_BUCKET_SOURCE = Join-Path $work 'main.zip'
    [void] (Invoke-GateModn @('bucket', 'update'))
  }
  $game = Join-Path $work 'game'
  Write-File (Join-Path $game 'bin/x64/Cyberpunk2077.exe') 'disposable marker'
  Write-File (Join-Path $game 'archive/pc/mod/manual.archive') 'manual sibling'
  $baseline = Snapshot $game
  Check 'Git catalog sync' {
    $catalog = Join-Path $work 'catalog'
    Write-File (Join-Path $catalog 'bucket.json') '{"schema_version":1,"name":"gate","title":"Gate"}'
    $manifest = Get-Content (Join-Path $repo 'buckets/main/games/cyberpunk-2077/redscript.json') -Raw | ConvertFrom-Json
    $manifest.source = [pscustomobject]@{ kind = 'github-release'; repository = 'jac3km4/redscript'; tag = 'v0.5.32'; asset = 'redscript-v0.5.32-windows.zip' }
    Write-File (Join-Path $catalog 'games/cyberpunk-2077/redscript.json') ($manifest | ConvertTo-Json -Depth 10)
    & git init $catalog
    Require ($LASTEXITCODE -eq 0) 'git init failed'
    & git -C $catalog add .
    Require ($LASTEXITCODE -eq 0) 'git add failed'
    & git -C $catalog -c user.name=Gate -c user.email=gate@example.invalid -c commit.gpgsign=false commit -m fixture
    Require ($LASTEXITCODE -eq 0) 'git commit failed'
    $revision = (& git -C $catalog rev-parse HEAD).Trim()
    Require ($LASTEXITCODE -eq 0) 'git rev-parse failed'
    [void] (Invoke-GateModn @('bucket', 'sync', 'gate', '--source', $catalog, '--revision', $revision))
    [void] (Invoke-GateModn @('bucket', 'sync', 'gate'))
    $list = @(Invoke-GateModn @('bucket', 'list'))
    Require (@($list | Where-Object name -eq 'gate').Count -eq 1) 'Synced catalog missing'
    $search = @(Invoke-GateModn @('search', 'redscript'))
    Require (@($search | Where-Object { $_.bucket -eq 'gate' -and $_.id -eq 'redscript' }).Count -eq 1) 'Synced package missing'
  }
  Check 'github-release install' {
    [void] (Invoke-GateModn @('install', 'gate/cyberpunk-2077/redscript', '--yes', '--allow-executable', 'engine/tools/scc.exe', '--path', $game))
    Require (Test-Path (Join-Path $game 'engine/tools/scc.exe')) 'Provider install missing executable'
    $packages = @(Invoke-GateModn @('list', '--path', $game))
    Require ($packages.Count -eq 1) 'Provider ownership missing'
    [void] (Invoke-GateModn @('uninstall', $packages[0].id, '--yes', '--path', $game))
    Require ((Snapshot $game) -eq $baseline) 'Provider uninstall changed manual bytes'
  }
  $adopt = Zip 'adopt' @{ 'archive/pc/mod/adopt.archive' = 'adopt me' }
  Write-File (Join-Path $game 'archive/pc/mod/adopt.archive') 'adopt me'
  Check 'Explicit adoption' {
    $before = Snapshot $game
    [void] (Invoke-GateModn @('adopt', $adopt, '--dry-run', '--path', $game))
    [void] (Invoke-GateModn @('adopt', $adopt, '--yes', '--path', $game) -Refuse)
    [void] (Invoke-GateModn @('adopt', $adopt, '--acknowledge', '--path', $game) -Refuse)
    Require ((Snapshot $game) -eq $before) 'Refused adoption changed bytes'
    $package = Invoke-GateModn @('adopt', $adopt, '--acknowledge', '--yes', '--path', $game)
    Require (@($package.files).Count -eq 1) 'Adoption did not claim exactly one file'
    Require ((Snapshot $game) -eq $before) 'Adoption changed game bytes'
  }
  Check 'Approved cleanup' {
    $packages = @(Invoke-GateModn @('list', '--path', $game))
    Require ($packages.Count -eq 1) 'Expected one adopted package'
    Write-File (Join-Path $game 'archive/pc/mod/adopt.archive') 'modified owned bytes'
    $before = Snapshot $game
    [void] (Invoke-GateModn @('uninstall', $packages[0].id, '--yes', '--path', $game) -Refuse)
    [void] (Invoke-GateModn @('uninstall', $packages[0].id, '--force', '--path', $game) -Refuse)
    Require ((Snapshot $game) -eq $before) 'Refused cleanup changed bytes'
    [void] (Invoke-GateModn @('uninstall', $packages[0].id, '--force', '--yes', '--path', $game))
    Require ((Snapshot $game) -eq $baseline) 'Cleanup changed manual bytes'
    Require (@(Invoke-GateModn @('list', '--path', $game)).Count -eq 0) 'Cleanup retained ownership'
  }
  Check 'REDmod content install and uninstall' {
    $zip = Zip 'redmod' @{ 'mods/Gate/info.json' = '{"name":"Gate"}'; 'mods/Gate/archives/gate.archive' = 'redmod bytes' }
    [void] (Invoke-GateModn @('install', $zip, '--yes', '--path', $game))
    Require ((Get-Content (Join-Path $game 'mods/Gate/archives/gate.archive') -Raw) -eq 'redmod bytes') 'REDmod bytes missing'
    $packages = @(Invoke-GateModn @('list', '--path', $game))
    Require ($packages.Count -eq 1 -and @($packages[0].files).Count -eq 2) 'REDmod ownership missing'
    [void] (Invoke-GateModn @('uninstall', $packages[0].id, '--yes', '--path', $game))
    Require ((Snapshot $game) -eq $baseline) 'REDmod uninstall changed manual bytes'
  }
  Check 'Newer minimum host version refusal' {
    $manifest = Get-Content (Join-Path $repo 'buckets/main/games/cyberpunk-2077/module.json') -Raw | ConvertFrom-Json
    $manifest | Add-Member -NotePropertyName min_modn_version -NotePropertyValue '99.0.0' -Force
    $env:MODN_MAIN_BUCKET_SOURCE = Zip 'future-main' @{
      'bucket.json' = (Get-Content (Join-Path $repo 'buckets/main/bucket.json') -Raw)
      'games/cyberpunk-2077/module.json' = ($manifest | ConvertTo-Json)
      'games/cyberpunk-2077/module.lua' = 'error("future module must not execute")'
    }
    $hash = (Get-FileHash $env:MODN_MAIN_BUCKET_SOURCE -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-File "$env:MODN_MAIN_BUCKET_SOURCE.sha256" "$hash *future-main.zip`n"
    $before = Snapshot (Join-Path $env:LOCALAPPDATA 'modn/buckets/main')
    $message = Invoke-GateModn @('bucket', 'update') -Refuse
    Write-Host $message
    Require ($message -match '99\.0\.0' -and $message -match 'needs Modn' -and $message -notmatch 'unknown field') 'Missing minimum-version diagnostic'
    Require ((Snapshot (Join-Path $env:LOCALAPPDATA 'modn/buckets/main')) -eq $before) 'Refused bucket replaced main'
    $env:MODN_MAIN_BUCKET_SOURCE = Join-Path $work 'main.zip'
  }
  Check 'MCP verifier' {
    $server = Join-Path $VerifierBinaries 'modn-mcp.exe'
    $fault = Join-Path $VerifierBinaries 'modn-mcp-fault.exe'
    Require ((Test-Path $server) -and (Test-Path $fault)) 'Prebuild modn-mcp.exe and modn-mcp-fault.exe; release assets omit them'
    Run-Python '.agents/skills/verify-modn-mcp/verify.py' @('--server', $server, '--backend', $script:modn, '--fault-backend', $fault, '--evidence', (Join-Path $work 'mcp-evidence'))
    $summary = Get-Content (Join-Path $work 'mcp-evidence/summary.json') -Raw | ConvertFrom-Json
    Require ($summary.failures -eq 0 -and $summary.count -gt 0) 'MCP verifier reported no passing checks'
  }
  Check 'Desktop verifier' {
    $app = Join-Path $VerifierBinaries 'modn-desktop.exe'
    $backend = Join-Path $VerifierBinaries 'modn-desktop-test-backend.exe'
    Require ((Test-Path $app) -and (Test-Path $backend)) 'Prebuild desktop app and verification backend; release assets omit them'
    & agent-desktop skills get desktop | Set-Content (Join-Path $work 'desktop-skill.json')
    Require ($LASTEXITCODE -eq 0) 'agent-desktop desktop instructions unavailable'
    & agent-desktop skills get platform | Set-Content (Join-Path $work 'platform-skill.json')
    Require ($LASTEXITCODE -eq 0) 'agent-desktop platform instructions unavailable'
    $status = & agent-desktop status | ConvertFrom-Json
    Require ($LASTEXITCODE -eq 0 -and $status.ok) 'Desktop automation is unavailable'
    Run-Desktop '.agents/skills/verify-modn-desktop/scripts/verify.py' @('--app', $app, '--backend', $script:modn)
    Run-Desktop '.agents/skills/verify-modn-desktop/scripts/lifecycle.py' @('--app', $app, '--backend', $backend)
  }
} catch {
  $results.Add([pscustomobject]@{ name = 'Gate setup'; status = 'FAIL'; error = "$_" })
  Write-Host "FAIL Gate setup: $_"
} finally {
  Pop-Location
  foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
  Check 'Original Modn roots unchanged' {
    $afterRoots = @{}
    foreach ($path in $roots.Keys) {
      $after = Snapshot $path
      $afterRoots[$path] = $after
      $same = $roots[$path] -eq $after
      Write-Host "Root $path before=$($roots[$path].Length) characters after=$($after.Length) characters unchanged=$same"
      Require $same "Original root changed: $path"
    }
    @(Root-Report $afterRoots) | ConvertTo-Json | Set-Content (Join-Path $work 'roots-after.json')
  }
  $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $work 'summary.json')
  $results | Format-Table name, status, error -Wrap | Out-Host
  $failed = @($results | Where-Object status -eq 'FAIL').Count
  Write-Host "Summary: $($results.Count) checks, $failed failures. Evidence: $work"
}
if ($failed) { exit 1 }
