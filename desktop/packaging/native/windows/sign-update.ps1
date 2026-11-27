# The updater signature gelabber-native-windows-x64-setup.exe.sig, bound to the
# app's version, next to the installer in the current directory. RELEASE_TAG
# set: with TAURI_SIGNING_PRIVATE_KEY (and its password); otherwise with the
# test key updater-key.ps1 made (GELABBER_UPDATER_TEST_KEY).
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$name = 'gelabber-native-windows-x64-setup.exe'
$version = (Select-String -Path desktop\Cargo.toml -Pattern '^version = "(.+)"\s*$' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($env:RELEASE_TAG) {
  if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    throw "release $($env:RELEASE_TAG): the secret TAURI_SIGNING_PRIVATE_KEY is not set, so the update cannot be signed (desktop/README.md, Updates)"
  }
  npx.cmd --yes '@tauri-apps/cli@2.12.3' signer sign --app-version $version $name
} else {
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  npx.cmd --yes '@tauri-apps/cli@2.12.3' signer sign --app-version $version --private-key-path $env:GELABBER_UPDATER_TEST_KEY --password test $name
}
if (-not (Test-Path "$name.sig")) { throw "no signature $name.sig" }
