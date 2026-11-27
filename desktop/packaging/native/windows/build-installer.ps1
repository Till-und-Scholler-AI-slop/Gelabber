# The native app's installer from installer.nsi: stages gelabber.exe and
# gelabber_media.dll of $CARGO_TARGET_DIR\release with the icons, README and
# notices, and writes gelabber-native-windows-x64-setup.exe and its .sha256
# into the current directory (the repository root). NSIS from the machine,
# else from Chocolatey. DESKTOP_VERSION is the app's version (version.sh).
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$makensis = (Get-Command makensis.exe -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe' }
if (-not (Test-Path $makensis)) {
  choco install nsis --yes --no-progress
  $makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
}
& $makensis /VERSION
# desktop/packaging/native/version.sh
$version = $env:DESKTOP_VERSION
if (-not $version) { throw 'DESKTOP_VERSION is not set (desktop/packaging/native/version.sh)' }
$release = Join-Path $env:CARGO_TARGET_DIR 'release'
$stage = (New-Item -ItemType Directory -Force (Join-Path $env:RUNNER_TEMP 'native-installer')).FullName
Copy-Item (Join-Path $release 'gelabber.exe'), (Join-Path $release 'gelabber_media.dll') $stage
Copy-Item desktop\packaging\icons\icon.ico, desktop\packaging\icons\icon.png, desktop\packaging\native\THIRD-PARTY-NOTICES.txt $stage
Copy-Item desktop\packaging\native\README-windows.txt (Join-Path $stage 'README.txt')
$name = 'gelabber-native-windows-x64-setup.exe'
# Windows wants four numbers in the file version: no pre-release part.
$numeric = "$(($version -split '[-+]')[0]).0"
& $makensis /V3 "/DVERSION=$version" "/DFILEVERSION=$numeric" "/DSOURCE=$stage" "/DOUTFILE=$(Join-Path $PWD $name)" desktop\packaging\native\windows\installer.nsi
$hash = (Get-FileHash $name -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $PWD "$name.sha256"), "$hash  $name`n")
@(
  '### Native app (Windows x64)', '',
  '| file | size | sha256 |', '|---|---|---|',
  "| $name | $([math]::Round((Get-Item $name).Length / 1MB, 1)) MiB | $hash |"
) | Add-Content $env:GITHUB_STEP_SUMMARY
