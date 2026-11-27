# The native app on this machine's desktop, as smoke.sh does on Linux: started
# with an empty settings folder and GELABBER_SMOKE=10, it must end by itself
# with exit status 0 once it painted its setup screen. Run from the repository
# root; $CARGO_TARGET_DIR\release\gelabber.exe is the app.
$ErrorActionPreference = 'Stop'
$env:GELABBER_CONFIG_DIR = (New-Item -ItemType Directory -Force (Join-Path $env:RUNNER_TEMP 'native-smoke')).FullName
$env:GELABBER_SMOKE = '10'
$app = Start-Process -FilePath (Join-Path $env:CARGO_TARGET_DIR 'release\gelabber.exe') -PassThru
$null = $app.Handle
if (-not $app.WaitForExit(2 * 60 * 1000)) {
  $app.Kill()
  throw 'the native app did not end within 2 minutes'
}
if ($app.ExitCode -ne 0) { throw "the native app ended with exit code $($app.ExitCode) instead of painting its setup screen" }
