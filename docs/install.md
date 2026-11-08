# Install

## Install on Windows

Run this command in PowerShell without administrator rights:

```powershell
irm https://github.com/modn-org/modn/releases/latest/download/install.ps1 | iex
```

The script verifies and installs `modn.exe`, adds it to your user PATH, and runs
`modn bucket update`. That command verifies the signed `main` bucket with the
signer key built into Modn. Open a new terminal, then run `modn search`.
The executable embeds no game modules or bucket manifests. It links
the C runtime statically, so it does not need the Visual C++ Redistributable.
Check the download in PowerShell before you run it:

```powershell
(Get-FileHash .\modn.exe -Algorithm SHA256).Hash
Get-Content .\modn.exe.sha256
```
