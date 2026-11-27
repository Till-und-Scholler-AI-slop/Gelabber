# The key the Windows installer's update signature is checked with. Run from
# the repository root. RELEASE_TAG set: desktop/packaging/updater.pub, and only
# from the release's tag. CANDIDATE=true: updater.pub, signed once the tag
# exists (desktop-release.yml). Otherwise a test key of this run, which it
# writes to GITHUB_ENV (GELABBER_UPDATER_PUBKEY, GELABBER_UPDATER_TEST_KEY).
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if ($env:RELEASE_TAG) {
  # Before anything is signed with the release key: only the
  # released source gets its signature, one installer per version.
  if ($env:GITHUB_REF -ne "refs/tags/$($env:RELEASE_TAG)") {
    throw "release $($env:RELEASE_TAG) must be built from its tag, not $($env:GITHUB_REF)"
  }
  $pub = 'desktop\packaging\updater.pub'
  if (-not (Test-Path $pub) -or -not (Get-Content -Raw $pub).Trim()) {
    throw "release $($env:RELEASE_TAG): $pub is missing, and with it the key the app checks updates with (desktop/README.md, Updates)"
  }
  "release: the key of $pub"
} elseif ($env:CANDIDATE -eq 'true') {
  # The release's installer, with the key of updater.pub; desktop-release.yml signs it.
  "candidate: the key of desktop\packaging\updater.pub, signed once the tag exists"
} else {
  $key = Join-Path $env:RUNNER_TEMP 'updater-test.key'
  npx.cmd --yes '@tauri-apps/cli@2.12.3' signer generate --ci --password test --write-keys $key --force
  Add-Content $env:GITHUB_ENV "GELABBER_UPDATER_PUBKEY=$((Get-Content -Raw "$key.pub").Trim())"
  Add-Content $env:GITHUB_ENV "GELABBER_UPDATER_TEST_KEY=$key"
  "test key: $key"
}
