# PSScriptAnalyzer settings for the packaging scripts (lint.sh).
@{
  Severity = @('Error', 'Warning')
  # The scripts run in CI, where the host is the log: a function whose
  # output is its value writes its notes there (loopback-address.ps1).
  ExcludeRules = @('PSAvoidUsingWriteHost')
}
