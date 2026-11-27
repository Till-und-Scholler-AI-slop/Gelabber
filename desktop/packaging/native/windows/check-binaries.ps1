# What ships, read with dumpbin from $CARGO_TARGET_DIR\release: gelabber_media.dll
# exports gm_* only, holds no OpenH264/FFmpeg H264 code and delay-loads
# mfplat.dll, and neither it nor gelabber.exe needs more than a clean Windows
# 10/11 has (System32 DLLs, API sets, UCRT; no VC++ runtime, no debug CRT).
# Writes the sizes and dependencies to GITHUB_STEP_SUMMARY; fails on a finding.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$release = Join-Path $env:CARGO_TARGET_DIR 'release'
$dll = Get-Item (Join-Path $release 'gelabber_media.dll')
$exe = Get-Item (Join-Path $release 'gelabber.exe')
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$dumpbin = & $vswhere -latest -products '*' -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' |
  Sort-Object | Select-Object -Last 1
if (-not $dumpbin) { throw 'no dumpbin.exe in the Visual Studio installation' }

# Rows "ordinal hint RVA name" of the export table.
$listing = & $dumpbin /nologo /exports $dll.FullName
$exports = @($listing | ForEach-Object {
    if ($_ -match '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+(\S+)') { $Matches[1] }
  })
$names = $listing | Select-String '^\s+(\d+) number of names' | Select-Object -First 1
if (-not $names -or $exports.Count -ne [int]$names.Matches[0].Groups[1].Value) {
  $listing
  throw "could not read the export table of gelabber_media.dll ($($exports.Count) names found)"
}
$foreign = @($exports | Where-Object { $_ -notmatch '^gm_' })

function Needs($file) {
  & $dumpbin /nologo /dependents $file |
    ForEach-Object { if ($_ -match '^\s+(\S+\.(dll|drv))\s*$') { $Matches[1].ToLowerInvariant() } } |
    Sort-Object -Unique
}
$redistributable = '^(vcruntime|msvcp|msvcr|concrt|vccorlib|vcomp|vcamp|mfc|atl)\d|^ucrtbased\.'
function Missing($needs, $shipped) {
  $needs | Where-Object {
    $_ -match $redistributable -or -not (
      $_ -match '^(api|ext)-ms-win-' -or $shipped -contains $_ -or
      (Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$_")))
  }
}
$dllNeeds = @(Needs $dll.FullName)
$exeNeeds = @(Needs $exe.FullName)
$dllMissing = @(Missing $dllNeeds @())
# Windows N lacks mfplat.dll without the Media Feature Pack: the
# core loads it only when it looks for H264 (/DELAYLOAD).
$delayed = (& $dumpbin /nologo /dependents $dll.FullName | Out-String) -match '(?s)delay load dependencies:.*\bmfplat\.dll'
$exeMissing = @(Missing $exeNeeds @('gelabber_media.dll'))

$content = [Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($dll.FullName))
# Names from OpenH264, from libwebrtc's wrappers around it and FFmpeg,
# and from FFmpeg. "OpenH264" alone is no sign: the encoder factory's
# type name has it in every build.
$h264 = @('CWelsH264SVCEncoder', 'WelsInitEncoderExt', 'H264EncoderImpl', 'H264DecoderImpl',
    'libavcodec', 'avcodec_open2', 'third_party/ffmpeg') |
  Where-Object { $content.Contains($_) }

@(
  '### Native core and app (Windows x64)', '',
  '| file | size | needs |', '|---|---|---|',
  "| gelabber_media.dll | $([math]::Round($dll.Length / 1MB, 1)) MiB | $($dllNeeds -join ', ') |",
  "| gelabber.exe | $([math]::Round($exe.Length / 1MB, 1)) MiB | $($exeNeeds -join ', ') |", '',
  "Exports of gelabber_media.dll: $($exports.Count)"
) | Tee-Object -FilePath $env:GITHUB_STEP_SUMMARY -Append

$problems = @(
  if ($foreign) { "gelabber_media.dll exports more than gm_*: $($foreign -join ', ')" }
  if ($h264) { "gelabber_media.dll carries H264 code of OpenH264 or FFmpeg: $($h264 -join ', ')" }
  if (-not $dllNeeds -or $exeNeeds -notcontains 'gelabber_media.dll') { 'could not read the dependencies with dumpbin' }
  if ($dllMissing) { "gelabber_media.dll needs what a clean Windows lacks: $($dllMissing -join ', ')" }
  if (-not $delayed) { 'gelabber_media.dll does not delay-load mfplat.dll' }
  if ($exeMissing) { "gelabber.exe needs what a clean Windows lacks: $($exeMissing -join ', ')" }
)
if ($problems) {
  # One annotation each: PowerShell folds a long error message.
  $problems | ForEach-Object { "::error::$_" }
  throw "$($problems.Count) of the checks on the binaries failed"
}
