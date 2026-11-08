[CmdletBinding(DefaultParameterSetName = 'Release')]
param(
  [Parameter(ParameterSetName = 'Release')] [string] $Tag = 'v0.2.0',
  [Parameter(Mandatory, ParameterSetName = 'Local')] [string] $Binary,
  [Parameter(Mandatory, ParameterSetName = 'Local')] [string] $Checksum,
  # A `test-main-key` build used only for `bucket update` from a signed local bucket.
  [string] $BucketBinary,
  [string] $PreviousTag = 'v0.1.0',
  [string] $Repository = 'modn-org/modn'
)

# Runs published assets by default. Binary and Checksum select a local build.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-That([bool] $Condition, [string] $Message) {
  if (-not $Condition) { throw "gate failed: $Message" }
  Write-Host "ok: $Message"
}

function Get-TreeSnapshot([string] $Root) {
  @(Get-ChildItem -LiteralPath $Root -Recurse -Force | Sort-Object FullName | ForEach-Object {
    if ($_.PSIsContainer) { "$($_.FullName)|directory" }
    else { "$($_.FullName)|$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)" }
  })
}

function Test-SameTree($Before, $After) {
  -not (Compare-Object @($Before) @($After))
}

function Get-Download([string] $Url, [string] $Destination) {
  Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
}

function Get-ExpectedHash([string] $ChecksumFile) {
  ((Get-Content -LiteralPath $ChecksumFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
}

$script:Modn = $null
function Invoke-Modn([string[]] $Arguments, [switch] $AllowFailure, [string] $Executable = '') {
  if (-not $Executable) { $Executable = $script:Modn }
  $text = (& $Executable --json --state-dir $script:State --game cyberpunk-2077 @Arguments 2>&1 | Out-String)
  $exit = $LASTEXITCODE
  if ($exit -ne 0 -and -not $AllowFailure) { throw "modn $($Arguments -join ' ') exited with ${exit}: $text" }
  $json = $null
  if ($exit -eq 0 -and ($text.Trim().StartsWith('{') -or $text.Trim().StartsWith('['))) {
    try { $json = $text | ConvertFrom-Json } catch { $json = $null }
  }
  [pscustomobject]@{ Exit = $exit; Text = $text; Json = $json }
}

function New-GateZip([string] $Work, [string] $Name, [hashtable] $Files) {
  $root = Join-Path $Work "zip-$Name"
  $mod = Join-Path $root 'archive\pc\mod'
  New-Item -ItemType Directory -Path $mod -Force | Out-Null
  foreach ($file in $Files.Keys) {
    Set-Content -LiteralPath (Join-Path $mod $file) -Value $Files[$file] -NoNewline
  }
  $zip = Join-Path $Work "$Name.zip"
  Compress-Archive -Path (Join-Path $root 'archive') -DestinationPath $zip
  [pscustomobject]@{ Path = $zip; Sha256 = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Write-GateBucket([string] $Bucket, [string] $Current, $Releases) {
  $games = Join-Path $Bucket 'games\cyberpunk-2077'
  if (Test-Path -LiteralPath $games) { Remove-Item -LiteralPath $games -Recurse -Force }
  New-Item -ItemType Directory -Path $games -Force | Out-Null
  Set-Content -LiteralPath (Join-Path $Bucket 'bucket.json') `
    -Value '{"schema_version":1,"name":"gate","title":"Gate bucket"}'
  foreach ($version in $Releases.Keys) {
    $manifest = [ordered]@{
      schema_version = 1; id = 'gate-mod'; game = 'cyberpunk-2077'; name = 'Gate mod'
      version = $version; archive_format = 'zip'; sha256 = $Releases[$version].Sha256
      source = [ordered]@{ kind = 'manual'; filename = "gate-mod-$version.zip" }
    }
    $file = if ($version -eq $Current) { 'gate-mod.json' } else { "gate-mod@$version.json" }
    Set-Content -LiteralPath (Join-Path $games $file) -Value ($manifest | ConvertTo-Json -Depth 5)
  }
}

function Get-GameFile([string] $Game, [string] $Relative) {
  $path = Join-Path $Game $Relative
  if (Test-Path -LiteralPath $path) { Get-Content -LiteralPath $path -Raw } else { $null }
}

function Get-GatePackage {
  @((Invoke-Modn @('list', '--path', $script:Game)).Json | Where-Object { $_.catalog.mod_id -eq 'gate-mod' })
}

$previousLocalAppData = $env:LOCALAPPDATA
$previousBucketSource = $env:MODN_MAIN_BUCKET_SOURCE
$previousPath = $env:Path
$localBinary = if ($Binary) { (Resolve-Path -LiteralPath $Binary).Path }
$localChecksum = if ($Checksum) { (Resolve-Path -LiteralPath $Checksum).Path }
$bucketBinary = if ($BucketBinary) { (Resolve-Path -LiteralPath $BucketBinary).Path } else { $localBinary }
if ($localBinary) { $Tag = 'local binary' }
if (-not $BucketBinary) { $env:MODN_MAIN_BUCKET_SOURCE = $null }
$work = Join-Path ([IO.Path]::GetTempPath()) "modn-gate-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $work | Out-Null
. (Join-Path $PSScriptRoot 'gate-git.ps1')
$savedGit = Enter-GateGitEnvironment $work
try {
  Push-Location -LiteralPath $work
  $env:LOCALAPPDATA = Join-Path $work 'data'
  $script:State = Join-Path $work 'state'

  Write-Host '== Clean machine =='
  $env:Path = ($env:Path -split ';' | Where-Object {
      $_ -and -not (Test-Path -LiteralPath (Join-Path $_ 'cargo.exe')) -and
        -not (Test-Path -LiteralPath (Join-Path $_ 'rustc.exe'))
    }) -join ';'
  Assert-That (-not (Get-Command cargo, rustc -ErrorAction SilentlyContinue)) 'cargo and rustc are not on PATH'
  $insideRepository = (Get-Command git -ErrorAction SilentlyContinue) -and
    ($null -ne (git rev-parse --show-toplevel 2>$null))
  Assert-That (-not $insideRepository) "work directory $work is outside any Git repository"

  Write-Host "== Released assets of $Tag =="
  $base = "https://github.com/$Repository/releases/download/$Tag"
  $assets = New-Item -ItemType Directory -Path (Join-Path $work 'assets')
  if ($localBinary) {
    Copy-Item -LiteralPath $localBinary -Destination (Join-Path $assets 'modn.exe')
    Copy-Item -LiteralPath $localChecksum -Destination (Join-Path $assets 'modn.exe.sha256')
  } else {
    foreach ($name in @('modn.exe', 'modn.exe.sha256', 'install.ps1')) {
      Get-Download "$base/$name" (Join-Path $assets $name)
    }
  }
  $released = Join-Path $assets 'modn.exe'
  $releasedHash = (Get-FileHash -LiteralPath $released -Algorithm SHA256).Hash.ToLowerInvariant()
  Assert-That ($releasedHash -eq (Get-ExpectedHash (Join-Path $assets 'modn.exe.sha256'))) `
    "downloaded modn.exe matches its published SHA-256 ($releasedHash)"

  Write-Host '== Bootstrap with the released install script =='
  $bin = Join-Path $work 'bin'
  if ($localBinary) {
    New-Item -ItemType Directory -Path $bin | Out-Null
    Copy-Item -LiteralPath $released -Destination (Join-Path $bin 'modn.exe')
    $bootstrap = if ($BucketBinary) { $bucketBinary } else { Join-Path $bin 'modn.exe' }
    & $bootstrap bucket update
    Assert-That ($LASTEXITCODE -eq 0) 'local binary bootstrapped the main bucket'
  } else {
    & (Join-Path $assets 'install.ps1') -InstallDir $bin -ReleaseBase $base -NoModifyPath
  }
  $script:Modn = Join-Path $bin 'modn.exe'
  Assert-That ((Get-FileHash -LiteralPath $script:Modn -Algorithm SHA256).Hash.ToLowerInvariant() -eq $releasedHash) `
    'the installed modn.exe is the released file'
  $buckets = Invoke-Modn @('bucket', 'list')
  Assert-That (@($buckets.Json | Where-Object name -eq 'main').Count -eq 1) 'the install script bootstrapped the main bucket'
  $search = @((Invoke-Modn @('search')).Json)
  Assert-That ($search.Count -ge 13 -and @($search | Where-Object id -eq 'archivexl').Count -eq 1) `
    "search lists the bootstrapped catalog ($($search.Count) mods)"
  $again = Invoke-Modn @('bucket', 'update') -Executable $(if ($BucketBinary) { $bucketBinary } else { $script:Modn })
  Assert-That ($again.Json.bucket -eq 'main' -and -not $again.Json.changed) `
    "a second bucket update verifies the same release asset and changes nothing (digest $($again.Json.digest))"

  Write-Host '== Prerequisite stack on a fake game =='
  $game = Join-Path $work 'Cyberpunk 2077'
  New-Item -ItemType Directory -Path (Join-Path $game 'bin\x64') | Out-Null
  Set-Content -LiteralPath (Join-Path $game 'bin\x64\Cyberpunk2077.exe') -Value 'fixture'
  $script:Game = $game
  $emptyGame = Get-TreeSnapshot $game
  $preview = Invoke-Modn @('install', 'archivexl', '--dry-run', '--path', $game)
  Assert-That (@($preview.Json.dependencies | ForEach-Object reference) -contains 'main/cyberpunk-2077/red4ext' -and
      @($preview.Json.dependencies | ForEach-Object reference) -contains 'main/cyberpunk-2077/redscript') `
    'install archivexl --dry-run plans red4ext and redscript first'
  Assert-That (Test-SameTree $emptyGame (Get-TreeSnapshot $game)) 'the dry run left the fake game unchanged'
  $stack = Invoke-Modn @('install', 'archivexl', '--yes', '--allow-executable', 'engine/tools/scc.exe', '--path', $game)
  $installed = @($stack.Json.stack | ForEach-Object { $_.installed.catalog.mod_id })
  Assert-That ($installed.Count -eq 3 -and $installed[0] -eq 'red4ext' -and $installed[2] -eq 'archivexl') `
    "the stack installed in order ($($installed -join ', '))"
  foreach ($file in @('red4ext\RED4ext.dll', 'engine\tools\scc.exe', 'red4ext\plugins\ArchiveXL\ArchiveXL.dll')) {
    Assert-That (Test-Path -LiteralPath (Join-Path $game $file)) "the stack wrote $file"
  }
  $owned = @((Invoke-Modn @('list', '--path', $game)).Json | ForEach-Object { $_.catalog.mod_id })
  Assert-That ($owned.Count -eq 3) "list reports exactly the three owned packages ($($owned -join ', '))"

  Write-Host '== Update, rollback and pin with a local bucket =='
  $v1 = New-GateZip $work 'v1' @{ 'gate.archive' = 'v1' }
  $v2 = New-GateZip $work 'v2' @{ 'gate.archive' = 'v2'; 'gate-extra.archive' = 'v2' }
  $v3 = New-GateZip $work 'v3' @{ 'gate.archive' = 'v3'; 'gate-new.archive' = 'v3' }
  $bucket = Join-Path $work 'gate-bucket'
  Write-GateBucket $bucket '1.0.0' ([ordered]@{ '1.0.0' = $v1 })
  [void] (Invoke-Modn @('bucket', 'add', 'gate', $bucket))
  [void] (Invoke-Modn @('install', 'gate-mod', '--file', $v1.Path, '--yes', '--path', $game))
  Assert-That ((Get-GameFile $game 'archive\pc\mod\gate.archive') -eq 'v1') 'gate-mod 1.0.0 is installed'

  Write-GateBucket $bucket '2.0.0' ([ordered]@{ '2.0.0' = $v2; '1.0.0' = $v1 })
  $before = Get-TreeSnapshot $game
  $update = Invoke-Modn @('update', 'gate-mod', '--dry-run', '--file', $v2.Path, '--path', $game)
  Assert-That ($update.Text -match '1\.0\.0' -and $update.Text -match '2\.0\.0') `
    'the update preview names the installed and the target release'
  Assert-That (Test-SameTree $before (Get-TreeSnapshot $game)) 'the update preview left the game unchanged'
  [void] (Invoke-Modn @('update', 'gate-mod', '--yes', '--file', $v2.Path, '--path', $game))
  Assert-That ((Get-GameFile $game 'archive\pc\mod\gate.archive') -eq 'v2' -and
      (Test-Path -LiteralPath (Join-Path $game 'archive\pc\mod\gate-extra.archive'))) 'the update applied release 2.0.0'
  Assert-That ((Get-GatePackage)[0].archive_sha256 -eq $v2.Sha256) 'the recorded ownership names release 2.0.0'

  Write-GateBucket $bucket '3.0.0' ([ordered]@{ '3.0.0' = $v3; '2.0.0' = $v2; '1.0.0' = $v1 })
  $collision = Join-Path $game 'archive\pc\mod\gate-new.archive'
  Set-Content -LiteralPath $collision -Value 'unmanaged' -NoNewline
  $before = Get-TreeSnapshot $game
  $failed = Invoke-Modn @('update', 'gate-mod', '--yes', '--file', $v3.Path, '--path', $game) -AllowFailure
  Write-Host "failed update output: $($failed.Text.Trim())"
  Assert-That ($failed.Exit -ne 0) 'an update that meets an unmanaged file is refused'
  Assert-That (Test-SameTree $before (Get-TreeSnapshot $game)) `
    'the refused update changed nothing: 2.0.0 is intact and the unmanaged file is untouched'
  Remove-Item -LiteralPath $collision

  # A game file that another process holds open makes the update fail on the
  # file system, not in planning. The game must end up as it was.
  $held = Join-Path $game 'archive\pc\mod\gate-extra.archive'
  $before = Get-TreeSnapshot $game
  $lock = [IO.File]::Open($held, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
  try {
    $failed = Invoke-Modn @('update', 'gate-mod', '--yes', '--file', $v3.Path, '--path', $game) -AllowFailure
  } finally { $lock.Dispose() }
  Write-Host "failed update output: $($failed.Text.Trim())"
  Assert-That ($failed.Exit -ne 0) 'an update that cannot move a locked file of the old release fails'
  Assert-That (Test-SameTree $before (Get-TreeSnapshot $game)) `
    'the failed update rolled back: every file of release 2.0.0 is back and no file of 3.0.0 remains'
  Assert-That ((Get-GatePackage)[0].archive_sha256 -eq $v2.Sha256) 'ownership still names release 2.0.0 after the failure'
  $recover = Invoke-Modn @('recover', '--yes')
  Write-Host "recover output: $($recover.Text.Trim())"
  Assert-That (Test-SameTree $before (Get-TreeSnapshot $game)) 'recover finds nothing to repair and changes nothing'
  [void] (Invoke-Modn @('pin', 'gate-mod', '--path', $game))
  $before = Get-TreeSnapshot $game
  $pinned = Invoke-Modn @('update', 'gate-mod', '--yes', '--file', $v3.Path, '--path', $game) -AllowFailure
  Write-Host "pinned update output: $($pinned.Text.Trim())"
  Assert-That (Test-SameTree $before (Get-TreeSnapshot $game)) 'a pinned package is not updated: the game is unchanged'
  Assert-That ((Get-GatePackage)[0].pinned -and (Get-GatePackage)[0].archive_sha256 -eq $v2.Sha256) `
    'the pin is recorded and release 2.0.0 is still installed'
  [void] (Invoke-Modn @('unpin', 'gate-mod', '--path', $game))
  [void] (Invoke-Modn @('update', 'gate-mod', '--yes', '--file', $v3.Path, '--path', $game))
  Assert-That ((Get-GameFile $game 'archive\pc\mod\gate.archive') -eq 'v3') `
    'after unpin the same update applies release 3.0.0, so the pin was the only blocker'

  Write-Host "== State created by $PreviousTag opens under $Tag =="
  $oldAssets = New-Item -ItemType Directory -Path (Join-Path $work 'previous')
  $oldBase = "https://github.com/$Repository/releases/download/$PreviousTag"
  foreach ($name in @('modn.exe', 'modn.exe.sha256')) { Get-Download "$oldBase/$name" (Join-Path $oldAssets $name) }
  $oldModn = Join-Path $oldAssets 'modn.exe'
  Assert-That ((Get-FileHash -LiteralPath $oldModn -Algorithm SHA256).Hash.ToLowerInvariant() -eq
      (Get-ExpectedHash (Join-Path $oldAssets 'modn.exe.sha256'))) "downloaded $PreviousTag matches its published SHA-256"
  $oldState = Join-Path $work 'old-state'
  $oldGame = Join-Path $work 'Old Cyberpunk 2077'
  New-Item -ItemType Directory -Path (Join-Path $oldGame 'bin\x64') | Out-Null
  Set-Content -LiteralPath (Join-Path $oldGame 'bin\x64\Cyberpunk2077.exe') -Value 'fixture'
  $oldInstall = (& $oldModn --json --state-dir $oldState --game cyberpunk-2077 install $v1.Path --yes --path $oldGame 2>&1 | Out-String)
  Assert-That ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath (Join-Path $oldGame 'archive\pc\mod\gate.archive'))) `
    "$PreviousTag installed a local archive into its own state"
  $script:State = $oldState
  $oldList = @((Invoke-Modn @('list', '--path', $oldGame)).Json)
  Assert-That ($oldList.Count -eq 1) "$Tag lists the package that $PreviousTag installed"
  [void] (Invoke-Modn @('uninstall', $oldList[0].id, '--yes', '--path', $oldGame))
  Assert-That (-not (Test-Path -LiteralPath (Join-Path $oldGame 'archive\pc\mod\gate.archive'))) `
    "$Tag uninstalled the $PreviousTag package and removed its file"
} finally {
  Exit-GateGitEnvironment $savedGit
  $env:LOCALAPPDATA = $previousLocalAppData
  $env:MODN_MAIN_BUCKET_SOURCE = $previousBucketSource
  $env:Path = $previousPath
  Pop-Location
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "milestone 1 exit gate passed for $Tag"
