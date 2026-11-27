#!/usr/bin/env bash
# Lints the packaging scripts and desktop/native/scripts as the notices job of desktop-native.yml does:
# the shell scripts through shellcheck, the PowerShell ones through PSScriptAnalyzer (pwsh).
#
#   lint.sh [--install <dir>]
#
# With --install, it first downloads the pinned versions below into <dir>,
# checked against their SHA-256, and uses them; without, the ones on PATH.
set -euo pipefail
SHELLCHECK_VERSION=0.11.0
SHELLCHECK_SHA256=8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198
PSSCRIPTANALYZER_VERSION=1.25.0
PSSCRIPTANALYZER_SHA256=14e634c828eb98efb9f40b2918ba90f139ed5eccdf663a2a747736d996995d60

here="$(cd "$(dirname "$0")" && pwd)"
shellcheck=shellcheck
module=PSScriptAnalyzer
if [ "${1:-}" = "--install" ]; then
  tools="${2:?--install needs a directory}"
  mkdir -p "$tools"
  tools="$(cd "$tools" && pwd)"
  curl -fsSL "https://github.com/koalaman/shellcheck/releases/download/v${SHELLCHECK_VERSION}/shellcheck-v${SHELLCHECK_VERSION}.linux.x86_64.tar.xz" -o "$tools/shellcheck.tar.xz"
  echo "${SHELLCHECK_SHA256}  $tools/shellcheck.tar.xz" | sha256sum --check --quiet
  tar -xJf "$tools/shellcheck.tar.xz" -C "$tools" --strip-components=1 "shellcheck-v${SHELLCHECK_VERSION}/shellcheck"
  curl -fsSL "https://www.powershellgallery.com/api/v2/package/PSScriptAnalyzer/${PSSCRIPTANALYZER_VERSION}" -o "$tools/psscriptanalyzer.nupkg"
  echo "${PSSCRIPTANALYZER_SHA256}  $tools/psscriptanalyzer.nupkg" | sha256sum --check --quiet
  rm -rf "$tools/PSScriptAnalyzer"
  python3 -m zipfile -e "$tools/psscriptanalyzer.nupkg" "$tools/PSScriptAnalyzer"
  shellcheck="$tools/shellcheck"
  module="$tools/PSScriptAnalyzer/PSScriptAnalyzer.psd1"
fi

# The packaging scripts, desktop/native/scripts (libwebrtc, test desktop) and
# live-tests.sh (e2e-extended.yml).
cd "$here/.."
mapfile -t shell < <({ find packaging native/scripts -name '*.sh'; echo live-tests.sh; } | sort)
"$shellcheck" --version | sed -n 2p
"$shellcheck" --severity=warning "${shell[@]}"
echo "shellcheck: ${#shell[@]} scripts"
MODULE="$module" pwsh -NoProfile -NonInteractive -Command '
  $ErrorActionPreference = "Stop"
  Import-Module $env:MODULE
  "PSScriptAnalyzer $((Get-Module PSScriptAnalyzer).Version)"
  # The scripts and the data files beside them (the analyzer settings).
  $files = @(Get-ChildItem -Recurse -Include *.ps1, *.psd1 -Path packaging, native/scripts)
  $found = @($files | ForEach-Object { Invoke-ScriptAnalyzer -Path $_.FullName -Settings ./packaging/PSScriptAnalyzerSettings.psd1 })
  $found | Format-Table -AutoSize RuleName, Severity, ScriptName, Line, Message | Out-String -Width 400
  "PSScriptAnalyzer: $($files.Count) files ($(@($files | Where-Object Extension -eq '.ps1').Count) scripts), $($found.Count) findings"
  exit [int]($found.Count -gt 0)'
