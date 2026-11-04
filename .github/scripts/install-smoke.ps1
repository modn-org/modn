$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ("modn-install-smoke-" + [guid]::NewGuid().ToString('N'))
$previousData = $env:LOCALAPPDATA
$previousSource = $env:MODN_MAIN_BUCKET_SOURCE
$previousPath = $env:Path
$registryPath = 'HKCU:\Software\modn-install-smoke-' + [guid]::NewGuid().ToString('N')
$environmentKey = Join-Path $registryPath 'Environment'
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
  $script = Join-Path $root 'install.ps1'
  $tokens = $null
  $parseErrors = $null
  $ast = [Management.Automation.Language.Parser]::ParseFile($script, [ref] $tokens, [ref] $parseErrors)
  if ($parseErrors.Count) { throw 'Installer parsing failed' }
  $pathFunction = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Add-ModnUserPath' }, $false)
  . ([scriptblock]::Create($pathFunction.Extent.Text))
  New-Item -Path $environmentKey -Force | Out-Null
  $originalPath = '%SystemRoot%\system32;;C:\Keep Me;%USERPROFILE%\bin'
  New-ItemProperty -LiteralPath $environmentKey -Name Path -Value $originalPath -PropertyType ExpandString | Out-Null
  Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
  $key = Get-Item -LiteralPath $environmentKey
  try {
    $firstPath = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    Assert-That ($firstPath -ceq "$originalPath;$install") 'PATH preserves raw variables, empty entries and order'
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
    Assert-That ($key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) -ceq $firstPath) 'PATH second run changes nothing'
    Assert-That ($key.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::ExpandString) 'PATH retains ExpandString kind'
  } finally { $key.Dispose() }
  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($environmentKey.Substring(6), $true)
  try {
    $key.SetValue('Path', 'C:\Keep Me;;', [Microsoft.Win32.RegistryValueKind]::String)
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
    Assert-That ($key.GetValue('Path') -ceq "C:\Keep Me;;$install") 'PATH reuses trailing separator'
    Assert-That ($key.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::String) 'PATH retains String kind'
    $key.DeleteValue('Path')
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
    Assert-That ($key.GetValue('Path') -ceq $install) 'missing PATH starts with install directory'
    Assert-That ($key.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::ExpandString) 'missing PATH becomes ExpandString'
    $rawDuplicate = $install.ToUpperInvariant() + '\'
    $key.SetValue('Path', $rawDuplicate, [Microsoft.Win32.RegistryValueKind]::String)
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
    Assert-That ($key.GetValue('Path') -ceq $rawDuplicate) 'raw duplicate ignores case and trailing backslash'
    Assert-That ($key.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::String) 'raw duplicate preserves value kind'
    $expandedDuplicate = '%USERPROFILE%\modn-smoke-bin\'
    $key.SetValue('Path', $expandedDuplicate, [Microsoft.Win32.RegistryValueKind]::ExpandString)
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir (Join-Path $env:USERPROFILE 'modn-smoke-bin')
    Assert-That ($key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) -ceq $expandedDuplicate) 'expanded duplicate preserves raw PATH'
    $key.SetValue('Path', '', [Microsoft.Win32.RegistryValueKind]::String)
    Add-ModnUserPath -RegistryKeyPath $environmentKey -InstallDir $install
    Assert-That ($key.GetValue('Path') -ceq $install) 'empty PATH has no leading separator'
    Assert-That (($env:Path -split ';') -contains $install) 'current process PATH includes install directory'
  } finally { $key.Dispose() }
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
  $env:Path = $previousPath
  if (Test-Path -LiteralPath $registryPath) { Remove-Item -LiteralPath $registryPath -Recurse -Force }
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
