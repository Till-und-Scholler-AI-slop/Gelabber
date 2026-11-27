#!/usr/bin/env bash
# Lints the packaging scripts as the notices job of desktop-native.yml does:
# shellcheck for the shell scripts, PSScriptAnalyzer (pwsh) for PowerShell.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"
mapfile -t shell < <(find . -name '*.sh' | sort)
shellcheck --version | sed -n 2p
shellcheck --severity=warning "${shell[@]}"
echo "shellcheck: ${#shell[@]} scripts"
pwsh -NoProfile -NonInteractive -Command '
  $ErrorActionPreference = "Stop"
  Import-Module PSScriptAnalyzer
  "PSScriptAnalyzer $((Get-Module PSScriptAnalyzer).Version)"
  $found = @(Invoke-ScriptAnalyzer -Path . -Recurse -Settings ./PSScriptAnalyzerSettings.psd1)
  $found | Format-Table -AutoSize RuleName, Severity, ScriptName, Line, Message | Out-String -Width 400
  "PSScriptAnalyzer: $(@(Get-ChildItem -Recurse -Filter *.ps1).Count) scripts, $($found.Count) findings"
  exit [int]($found.Count -gt 0)'
