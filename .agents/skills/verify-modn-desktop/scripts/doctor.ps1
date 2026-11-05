param([Parameter(Mandatory=$true)][string]$Backend)
$ErrorActionPreference = 'Stop'
$checks = 0
function Confirm([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:checks++
}
Confirm ([Environment]::OSVersion.Version.Major -ge 10) 'Windows 10 or newer is required'
$rust = & rustc --version
Confirm ($LASTEXITCODE -eq 0 -and $rust -match 'rustc 1\.(\d+)' -and [int]$Matches[1] -ge 92 -and $rust -notmatch 'nightly') 'Stable Rust 1.92 or newer is required'
$cmake = & cmake --version
Confirm ($LASTEXITCODE -eq 0) 'CMake is required'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
Confirm (Test-Path -LiteralPath $vswhere) 'VS 2022 Build Tools are required'
$vs = & $vswhere -latest -version '[17.0,18.0)' -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
Confirm ($LASTEXITCODE -eq 0 -and [bool]$vs) 'VS 2022 C++ workload is required'
$sdk = Get-ChildItem -LiteralPath 'C:\Program Files (x86)\Windows Kits\10\Lib' -Directory | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'um\x64\user32.lib') }
Confirm ([bool]$sdk) 'Windows SDK x64 libraries are required'
$binary = (Resolve-Path -LiteralPath $Backend).Path
Confirm ([IO.Path]::IsPathRooted($binary) -and (Test-Path -LiteralPath $binary -PathType Leaf)) 'Choose an existing backend executable'
$version = & $binary --help
Confirm ($LASTEXITCODE -eq 0 -and [bool]($version -match 'Usage: modn')) 'The backend must identify itself as modn'
[pscustomobject]@{ checks=$checks; failures=0; rust=$rust; cmake=$cmake[0]; visual_studio=$vs; sdk=@($sdk.Name); backend=$binary; sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash; help=$version[0] } | ConvertTo-Json
exit 0
