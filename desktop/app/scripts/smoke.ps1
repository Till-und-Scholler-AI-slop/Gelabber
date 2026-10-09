# Starts the built desktop app on a stand-in server origin (smoke_server.py)
# and fails unless the page reached the media commands, and only those. The
# Windows counterpart of smoke.sh; needs python and the WebView2 Runtime.
# Usage: smoke.ps1 -App <path to gelabber-desktop.exe> -Abi <expected ABI version>
param(
  [Parameter(Mandatory = $true)][string]$App,
  [Parameter(Mandatory = $true)][int]$Abi
)
$ErrorActionPreference = 'Stop'
$port = 18099
$logs = Join-Path ([IO.Path]::GetTempPath()) "gelabber-smoke-$PID"
New-Item -ItemType Directory -Force -Path $logs | Out-Null

$server = Start-Process -FilePath python -PassThru -NoNewWindow `
  -ArgumentList '-I', "`"$PSScriptRoot\smoke_server.py`"", $port, $Abi `
  -RedirectStandardOutput "$logs\server.txt" -RedirectStandardError "$logs\server-error.txt"
# Without the handle taken now, the exit code is gone once the process ends.
$null = $server.Handle

$status = 1
$appProcess = $null
try {
  # The app's first request must find the server listening.
  $listening = $false
  foreach ($attempt in 1..50) {
    try {
      [Net.Sockets.TcpClient]::new('127.0.0.1', $port).Dispose()
      $listening = $true
      break
    } catch {
      if ($server.HasExited) { break }
      Start-Sleep -Milliseconds 200
    }
  }
  if (-not $listening) { throw 'smoke_server.py is not listening' }

  $env:GELABBER_SERVER = "http://127.0.0.1:$port"
  # Runners have no audio endpoints; the commands must answer all the same.
  $env:GELABBER_AUDIO = 'dummy'
  $appProcess = Start-Process -FilePath (Resolve-Path $App).Path -PassThru `
    -RedirectStandardOutput "$logs\app.txt" -RedirectStandardError "$logs\app-error.txt"
  if (-not $server.WaitForExit(120000)) { throw 'no report from the app within 120 s' }
  # Anything but a plain 0, a lost exit code included, is a failure.
  if ($server.ExitCode -eq 0) { $status = 0 }
} catch {
  "FAIL: $_"
} finally {
  if (-not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
  if ($appProcess) {
    # The app and the WebView2 processes below it.
    & taskkill.exe /PID $appProcess.Id /T /F | Out-Null
  }
  foreach ($log in 'server.txt', 'server-error.txt', 'app-error.txt') {
    if (Test-Path "$logs\$log") { Get-Content "$logs\$log" }
  }
}
exit $status
