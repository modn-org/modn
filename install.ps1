[CmdletBinding()]
param(
  [string] $InstallDir = (Join-Path $env:LOCALAPPDATA 'modn\bin'),
  [string] $ReleaseBase = 'https://github.com/modn-org/modn/releases/latest/download',
  [switch] $NoModifyPath
)

$ErrorActionPreference = 'Stop'
function Add-ModnUserPath([string] $RegistryKeyPath, [string] $InstallDir) {
  if (-not $RegistryKeyPath.StartsWith('HKCU:\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'PATH registry key must be under HKCU.'
  }
  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($RegistryKeyPath.Substring(6), $true)
  if ($null -eq $key) { throw 'PATH registry key does not exist.' }
  try {
    $userPath = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $normalized = $InstallDir.TrimEnd('\')
    foreach ($entry in ($userPath -split ';')) {
      if ($entry.TrimEnd('\') -ieq $normalized -or
          [Environment]::ExpandEnvironmentVariables($entry).TrimEnd('\') -ieq $normalized) {
        return
      }
    }
    $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
    if ($key.GetValueNames() -icontains 'Path') { $kind = $key.GetValueKind('Path') }
    $separator = ''
    if ($userPath.Length -gt 0 -and -not $userPath.EndsWith(';')) { $separator = ';' }
    $key.SetValue('Path', ($userPath + $separator + $InstallDir), $kind)
  } finally { $key.Dispose() }
  if (-not ('Modn.EnvironmentBroadcast' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace Modn {
    public static class EnvironmentBroadcast {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(
            IntPtr window, uint message, UIntPtr wParam, string lParam,
            uint flags, uint timeout, out UIntPtr result);
    }
}
'@
  }
  $result = [UIntPtr]::Zero
  [void] [Modn.EnvironmentBroadcast]::SendMessageTimeout(
    [IntPtr] 0xffff, 0x001a, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref] $result)
  $processSeparator = ''
  if ($env:Path -and -not $env:Path.EndsWith(';')) { $processSeparator = ';' }
  $env:Path = $env:Path + $processSeparator + $InstallDir
  Write-Host "Added $InstallDir to the user PATH."
}

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
    Add-ModnUserPath -RegistryKeyPath 'HKCU:\Environment' -InstallDir $InstallDir
  }
  $previousSource = $env:MODN_MAIN_BUCKET_SOURCE
  try {
    if ($local) { $env:MODN_MAIN_BUCKET_SOURCE = (Resolve-Path -LiteralPath (Join-Path $ReleaseBase 'main-bucket.zip')).Path }
    & $target bucket update
    if ($LASTEXITCODE -ne 0) { throw "modn bucket update failed with exit code $LASTEXITCODE." }
  } finally { $env:MODN_MAIN_BUCKET_SOURCE = $previousSource }
  Write-Host 'Run modn search to discover packages. Open a new terminal if PATH changed.'
} finally { Remove-Item -LiteralPath $work -Recurse -Force }
