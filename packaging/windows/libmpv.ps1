# Fetches libmpv for Windows (shinchiro's mpv-dev build) and makes the
# mpv.lib import library that MSVC links against. Needs gh, 7z, dumpbin and
# lib.exe on PATH (a GitHub Actions Windows runner after msvc-dev-cmd).
# Leaves libmpv-2.dll in .\mpv, adds it to PATH and to the linker search path.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true # a failing gh, 7z or lib stops the script

gh release download --repo shinchiro/mpv-winbuild-cmake --pattern 'mpv-dev-x86_64-2*.7z' --dir mpv-download
7z x 'mpv-download\*.7z' -ompv | Out-Null
$dll = Get-ChildItem mpv -Recurse -Filter 'libmpv*.dll' | Select-Object -First 1
if (-not $dll) { throw 'libmpv DLL not found in the mpv-dev archive' }

# dumpbin lists exports as "ordinal hint RVA name".
$names = dumpbin /nologo /exports $dll.FullName |
    Select-String '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]{8}\s+(\w+)' |
    ForEach-Object { $_.Matches[0].Groups[1].Value }
if (-not ($names -contains 'mpv_create')) { throw 'mpv_create missing from the DLL exports' }
Set-Content mpv\mpv.def (@('EXPORTS') + $names)
lib /nologo /def:mpv\mpv.def "/name:$($dll.Name)" /out:mpv\mpv.lib /machine:x64

$dir = (Resolve-Path mpv).Path
Copy-Item $dll.FullName $dir -ErrorAction SilentlyContinue
"RUSTFLAGS=-L native=$dir" >> $env:GITHUB_ENV
$dir >> $env:GITHUB_PATH
