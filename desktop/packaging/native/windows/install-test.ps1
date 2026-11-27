# gelabber-native-windows-x64-setup.exe (current directory) as a user runs it,
# then as the app's updater does (shortcuts kept, the app restarted with its
# arguments), then its uninstaller; each time it checks what is installed and
# what is left. Installs into this account: run it on a test machine only.
# $CARGO_TARGET_DIR\release\gelabber.exe is the binary the installer must hold.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$id = 'io.github.till-und-scholler-ai-slop.gelabber'
$install = Join-Path $env:LOCALAPPDATA 'Gelabber'
$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'Gelabber.lnk'
$desktop = Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'Gelabber.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Gelabber'
# desktop/packaging/native/version.sh
$version = $env:DESKTOP_VERSION
if (-not $version) { throw 'DESKTOP_VERSION is not set (desktop/packaging/native/version.sh)' }
function Run($file, [string[]]$arguments, $minutes) {
  $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru
  # Without the handle taken now, the exit code is gone once the process ends.
  $null = $process.Handle
  if (-not $process.WaitForExit($minutes * 60 * 1000)) {
    $process.Kill()
    throw "$file $arguments did not end within $minutes minutes"
  }
  if ($process.ExitCode -ne 0) { throw "$file $arguments ended with exit code $($process.ExitCode)" }
}
# The app this setup installed, with the shortcuts that lead to it.
function Installed($shortcuts) {
  foreach ($file in 'gelabber.exe', 'gelabber_media.dll', 'icon.png', 'icon.ico', 'README.txt', 'THIRD-PARTY-NOTICES.txt', 'uninstall.exe') {
    if (-not (Test-Path (Join-Path $install $file))) { throw "missing after the install: $file" }
  }
  $built = Get-FileHash (Join-Path $env:CARGO_TARGET_DIR 'release\gelabber.exe')
  if ((Get-FileHash (Join-Path $install 'gelabber.exe')).Hash -ne $built.Hash) {
    throw 'the installed gelabber.exe is not the one built here'
  }
  $entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
  if (-not $entry) { throw 'no entry among the installed apps (uninstall key of the account)' }
  if ($entry.DisplayVersion -ne $version) { throw "the installed apps list version $($entry.DisplayVersion), not $version" }
  "installed: $($entry.DisplayName) $($entry.DisplayVersion) in $install"
  foreach ($shortcut in $shortcuts) {
    if (-not (Test-Path $shortcut)) { throw "no shortcut $shortcut" }
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    if ($target -ne (Join-Path $install 'gelabber.exe')) { throw "$shortcut leads to $target" }
    $item = (New-Object -ComObject Shell.Application).NameSpace((Split-Path $shortcut)).ParseName((Split-Path $shortcut -Leaf))
    $aumid = $item.ExtendedProperty('System.AppUserModel.ID')
    if ($aumid -ne $id) { throw "$shortcut has the AppUserModelId '$aumid', not $id" }
  }
}
function Removed {
  $started = (Get-Date).ToUniversalTime()
  Start-Process -FilePath (Join-Path $install 'uninstall.exe') -ArgumentList '/S'
  $left = {
    @($install, $lnk, $desktop, $uninstallKey, "HKCU:\Software\Classes\AppUserModelId\$id") |
      Where-Object { Test-Path -LiteralPath $_ }
  }
  $deadline = (Get-Date).AddMinutes(3)
  while ((& $left) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
  $rest = @(& $left)
  # A write after the uninstaller's start means written again; else it could not delete the file, still held or held then (GEL-374).
  foreach ($file in @($rest | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })) {
    $item = Get-Item -LiteralPath $file
    "${file}: written $($item.LastWriteTimeUtc.ToString('o')), uninstaller started $($started.ToString('o'))"
    try { Remove-Item -LiteralPath $file -ErrorAction Stop; "${file}: deletes now" } catch { "${file}: does not delete now: $($_.Exception.Message)" }
  }
  if ($rest) { throw "left behind by the uninstaller: $($rest -join ', ')" }
  if (-not (Test-Path -LiteralPath 'HKCU:\Software\Gelabber\Gelabber')) {
    throw "the uninstaller removed the installer's note of the folder without being asked"
  }
}
$setup = (Resolve-Path gelabber-native-windows-x64-setup.exe).Path
Run $setup '/S' 10
Installed @($lnk, $desktop)
# The updater's arguments (update.rs): the restarted app ends by itself once it painted (GELABBER_SMOKE).
$env:GELABBER_SMOKE = '10'
Run $setup '/P', '/UPDATE', '/R', '/ARGS', '--server', '"https://gelabber.invalid"' 10
$restarted = @(Get-Process -Name gelabber -ErrorAction SilentlyContinue)
if (-not $restarted) { throw 'the update did not start gelabber.exe (/R)' }
foreach ($process in $restarted) { $null = $process.Handle }
foreach ($process in $restarted) {
  if (-not $process.WaitForExit(120 * 1000)) { throw 'the restarted gelabber.exe did not end within 2 minutes' }
  if ($process.ExitCode -ne 0) { throw "the restarted gelabber.exe ended with exit code $($process.ExitCode), not 0 (painted)" }
}
Remove-Item Env:GELABBER_SMOKE
Installed @($lnk, $desktop)
Removed
