# Build the pinned libwebrtc for Windows x64 and package it for the desktop
# media core.
#
#   pwsh desktop/native/scripts/build-libwebrtc-windows.ps1 <work-dir> <package-dir>
#
# Uses the local Visual Studio 2022 + Windows SDK (DEPOT_TOOLS_WIN_TOOLCHAIN=0),
# Chromium's clang-cl and the MSVC STL (use_custom_libcxx=false) with the
# static CRT (/MT), so MSVC-built code links against it directly.
param(
  [Parameter(Mandatory = $true)][string]$Work,
  [Parameter(Mandatory = $true)][string]$Package
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pins = @{}
Get-Content (Join-Path $here '..\libwebrtc.env') | Where-Object { $_ -match '^[A-Z_]+=' } | ForEach-Object {
  $k, $v = $_ -split '=', 2
  $pins[$k] = $v
}
$commit = $pins['WEBRTC_COMMIT']

New-Item -ItemType Directory -Force -Path $Work | Out-Null
Set-Location $Work
if (-not (Test-Path depot_tools)) {
  git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git
}
$env:PATH = "$Work\depot_tools;$env:PATH"
$env:DEPOT_TOOLS_WIN_TOOLCHAIN = '0'
$env:vs2022_install = (& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath)

# First run bootstraps depot_tools on Windows (git.bat, python3.bat via CIPD);
# gclient's git cache lookup fails without it.
gclient

@"
solutions = [{
  "name": "src",
  "url": "https://webrtc.googlesource.com/src.git@$commit",
  "deps_file": "DEPS",
  "managed": False,
  "custom_deps": {},
}]
target_os = ["win"]
"@ | Set-Content -Encoding ascii .gclient

gclient sync --no-history --shallow --nohooks -r "src@$commit" -j 8
gclient runhooks

Set-Location "$Work\src"
if ((git rev-parse HEAD) -ne $commit) { throw "unexpected libwebrtc commit" }
Get-ChildItem (Join-Path $here '..\patches\*.patch') -ErrorAction SilentlyContinue | ForEach-Object {
  git apply --check $_.FullName 2>$null
  if ($LASTEXITCODE -eq 0) { git apply $_.FullName } else { git apply --check --reverse $_.FullName }
}

$gnArgs = @(
  'target_os="win"', 'target_cpu="x64"',
  'is_debug=false', 'is_component_build=false', 'symbol_level=0',
  'treat_warnings_as_errors=false',
  'rtc_include_tests=false', 'rtc_build_examples=false', 'rtc_build_tools=false',
  'rtc_enable_protobuf=false', 'rtc_use_perfetto=false', 'use_rtti=true',
  'use_custom_libcxx=false', 'use_custom_libcxx_for_host=false',
  'rtc_use_h264=true',
  'enable_rust=false', 'enable_rust_cxx=false', 'enable_chromium_prelude=false', 'rtc_rusty_base64=false'
) -join ' '
gn gen out\gelabber "--args=$gnArgs"
ninja -C out\gelabber :default

if (Test-Path $Package) { Remove-Item -Recurse -Force $Package }
New-Item -ItemType Directory -Force -Path "$Package\lib", "$Package\include" | Out-Null
Copy-Item out\gelabber\obj\webrtc.lib "$Package\lib\webrtc.lib"
robocopy . "$Package\include" *.h *.hpp *.inc /S /XD out /NP /NFL /NDL | Out-Null
if ($LASTEXITCODE -ge 4) { throw "robocopy failed" }
$global:LASTEXITCODE = 0
Copy-Item out\gelabber\args.gn "$Package\args.gn"
@(
  "WEBRTC_BRANCH=$($pins['WEBRTC_BRANCH'])",
  "WEBRTC_COMMIT=$commit",
  "LIBWEBRTC_PACKAGE_REVISION=$($pins['LIBWEBRTC_PACKAGE_REVISION'])"
) | Set-Content -Encoding ascii "$Package\VERSIONS"
Get-ChildItem "$Package\lib" | Format-Table Name, Length
