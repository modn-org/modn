$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'gate-git.ps1')
$work = Join-Path ([IO.Path]::GetTempPath()) "modn-git-isolation-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $work | Out-Null
$savedGit = Enter-GateGitEnvironment $work
try {
  $outside = Join-Path $work 'outside'
  New-Item -ItemType Directory -Path $outside | Out-Null
  [IO.File]::WriteAllText((Join-Path $outside 'bucket.json'), 'staged outside bytes')
  [void] (New-GateGitCommit $outside)
  $index = Join-Path $outside '.git/index'
  $before = (Get-FileHash $index -Algorithm SHA256).Hash
  $sentinel = Join-Path $outside 'sentinel'
  $hooks = Join-Path $outside 'hostile-hooks'
  New-Item -ItemType Directory -Path $hooks | Out-Null
  $sentinelPath = $sentinel.Replace('\', '/')
  [IO.File]::WriteAllText((Join-Path $hooks 'pre-commit'), "#!/bin/sh`nprintf hostile > '$sentinelPath'`n")
  $env:GIT_INDEX_FILE = $index
  $env:GIT_CONFIG_COUNT = '1'
  $env:GIT_CONFIG_KEY_0 = 'core.hooksPath'
  $env:GIT_CONFIG_VALUE_0 = $hooks
  $fixture = Join-Path $work 'fixture'
  New-Item -ItemType Directory -Path $fixture | Out-Null
  [IO.File]::WriteAllText((Join-Path $fixture 'bucket.json'), 'fixture bytes')
  $hostileGit = Enter-GateGitEnvironment $fixture
  try { [void] (New-GateGitCommit $fixture) }
  finally { Exit-GateGitEnvironment $hostileGit }
  if ((Get-FileHash $index -Algorithm SHA256).Hash -ne $before) { throw 'Outside index changed' }
  if (Test-Path $sentinel) { throw 'Inherited hook ran outside fixture' }
  if ($env:GIT_INDEX_FILE -ne $index -or $env:GIT_CONFIG_VALUE_0 -ne $hooks) { throw 'Caller environment was not restored' }
  Write-Host 'PASS Git isolation: 3 assertions (index, hook sentinel, environment restoration)'
} finally {
  Exit-GateGitEnvironment $savedGit
  Remove-Item -LiteralPath $work -Recurse -Force
}

