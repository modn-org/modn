function Enter-GateGitEnvironment([string] $Root) {
  $savedGit = @{}
  foreach ($entry in Get-ChildItem Env: | Where-Object Name -like 'GIT_*') {
    $savedGit[$entry.Name] = $entry.Value
    Remove-Item -LiteralPath "Env:$($entry.Name)"
  }
  $config = Join-Path $Root 'isolated.gitconfig'
  $hooks = Join-Path $Root 'empty-hooks'
  New-Item -ItemType Directory -Path $hooks -Force | Out-Null
  $hookPath = $hooks.Replace('\', '/')
  [IO.File]::WriteAllText($config, "[core]`n`thooksPath = `"$hookPath`"`n")
  $env:GIT_CONFIG_GLOBAL = $config
  $env:GIT_CONFIG_NOSYSTEM = '1'
  return $savedGit
}

function Exit-GateGitEnvironment([hashtable] $SavedGit) {
  foreach ($entry in Get-ChildItem Env: | Where-Object Name -like 'GIT_*') {
    Remove-Item -LiteralPath "Env:$($entry.Name)"
  }
  foreach ($name in $SavedGit.Keys) {
    [Environment]::SetEnvironmentVariable($name, $SavedGit[$name])
  }
}

function New-GateGitCommit([string] $Catalog) {
  & git init $Catalog | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'git init failed' }
  & git -C $Catalog add . | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'git add failed' }
  & git -C $Catalog -c user.name=Gate -c user.email=gate@example.invalid -c commit.gpgsign=false commit -m fixture | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'git commit failed' }
  $revision = (& git -C $Catalog rev-parse HEAD).Trim()
  if ($LASTEXITCODE -ne 0) { throw 'git rev-parse failed' }
  return $revision
}

