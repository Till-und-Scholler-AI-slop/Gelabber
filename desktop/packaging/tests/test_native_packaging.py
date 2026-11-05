"""The native app's packages against what the app and the Tauri app say:
python3 -m unittest discover -s desktop/packaging/tests

The names only meet at run time (the desktop entry and the window's app id,
the installer and the Tauri app's folder and AppUserModelId), so a rename on
one side is caught here rather than on a user's desktop. makensis and
makepkg are not needed."""
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

DESKTOP = Path(__file__).parents[2]
NATIVE = DESKTOP / 'packaging/native'
NSIS = NATIVE / 'windows/installer.nsi'
PKGBUILD = DESKTOP / 'packaging/arch/native/PKGBUILD'


def defines(text):
    return dict(re.findall(r'^!define (\w+) "([^"]*)"', text, re.M))


def bash_array(pkgbuild, name):
    """A PKGBUILD array as bash reads it."""
    out = subprocess.run(['bash', '-c', f'source "$1"; printf "%s\\n" "${{{name}[@]}}"', '_', pkgbuild],
                         capture_output=True, text=True, check=True)
    return out.stdout.splitlines()


class DesktopEntry(unittest.TestCase):
    def test_the_entry_is_named_after_the_windows_app_id(self):
        platform = (DESKTOP / 'ui/src/platform/mod.rs').read_text()
        app_id = re.search(r'pub const APP_ID: &str = "([^"]+)";', platform).group(1)
        entry = (NATIVE / f'{app_id}.desktop').read_text()
        self.assertIn(f'\nIcon={app_id}\n', entry)
        self.assertIn(f'\nStartupWMClass={app_id}\n', entry)
        self.assertIn('\nExec=gelabber %U\n', entry)


class Installer(unittest.TestCase):
    def setUp(self):
        self.text = NSIS.read_text()
        self.defined = defines(self.text)

    def test_it_takes_the_tauri_apps_place(self):
        tauri = json.loads((DESKTOP / 'app/tauri.conf.json').read_text())
        bundle = json.loads((DESKTOP / 'app/tauri.bundle.windows.json').read_text())['bundle']
        self.assertEqual(self.defined['BUNDLEID'], tauri['identifier'])
        self.assertEqual(self.defined['PRODUCTNAME'], tauri['productName'])
        self.assertEqual(self.defined['MANUFACTURER'], tauri['bundle']['publisher'])
        # Tauri's currentUser mode: the account's folder, no administrator.
        self.assertEqual(bundle['windows']['nsis']['installMode'], 'currentUser')
        self.assertIn('InstallDir "$LOCALAPPDATA\\${PRODUCTNAME}"', self.text)
        self.assertIn('RequestExecutionLevel user', self.text)
        self.assertIn('Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${PRODUCTNAME}',
                      self.defined['UNINSTKEY'])
        self.assertIn('Delete "$INSTDIR\\${TAURIBINARY}"', self.text)
        self.assertEqual(self.defined['TAURIBINARY'], 'gelabber-desktop.exe')
        # hooks.nsh: the toasts' entry goes with the app.
        hooks = (DESKTOP / 'app/windows/hooks.nsh').read_text()
        self.assertIn('Software\\Classes\\AppUserModelId\\${BUNDLEID}', hooks)
        self.assertIn('DeleteRegKey HKCU "${AUMIDKEY}"', self.text)

    def test_it_accepts_the_tauri_updaters_arguments(self):
        # tauri-plugin-updater runs an NSIS setup with /P /UPDATE /R /ARGS.
        for option in ('/P', '/UPDATE', '/R', '/ARGS'):
            with self.subTest(option=option):
                self.assertIn(f'${{GetOptions}} $R0 "{option}" $R1', self.text)

    def test_it_installs_what_the_workflow_stages(self):
        workflow = (DESKTOP.parent / '.github/workflows/desktop-native.yml').read_text()
        files = re.findall(r'^\s+File "\$\{SOURCE\}\\([^"]+)"', self.text, re.M)
        files = [f.replace('${MAINBINARY}', self.defined['MAINBINARY']) for f in files]
        self.assertEqual(sorted(files), sorted([
            'gelabber.exe', 'gelabber_media.dll', 'icon.png', 'icon.ico', 'README.txt',
            'THIRD-PARTY-NOTICES.txt']))
        step = workflow.split('- name: Native installer\n', 1)[1].split('\n      - name:', 1)[0]
        for staged in ("'gelabber.exe'", "'gelabber_media.dll'", 'icon.ico', 'icon.png',
                       'THIRD-PARTY-NOTICES.txt', "'README.txt'"):
            with self.subTest(staged=staged):
                self.assertIn(staged, step)
        for removed in files + ['uninstall.exe', '${TAURIBINARY}']:
            name = removed if removed != self.defined['MAINBINARY'] else '${MAINBINARY}'
            with self.subTest(removed=removed):
                self.assertIn(f'Delete "$INSTDIR\\{name}"', self.text)


class ArchPackage(unittest.TestCase):
    def test_the_beta_package_replaces_the_tauri_one(self):
        self.assertIn('gelabber-desktop', bash_array(PKGBUILD, 'conflicts'))
        self.assertEqual(bash_array(PKGBUILD, 'pkgname'), ['gelabber-desktop-native'])

    def test_no_web_view_or_gtk_dependency(self):
        depends = bash_array(PKGBUILD, 'depends')
        for package in ('webkit2gtk-4.1', 'libsoup3', 'gtk3'):
            with self.subTest(package=package):
                self.assertNotIn(package, depends)

    def test_the_package_installs_what_the_tarball_holds(self):
        script = (NATIVE / 'tarball.sh').read_text()
        package = PKGBUILD.read_text().split('package() {', 1)[1]
        for name in ('gelabber', 'libgelabber_media.so', 'README.txt', 'THIRD-PARTY-NOTICES.txt',
                     'gelabber.desktop', 'gelabber.png'):
            with self.subTest(name=name):
                self.assertRegex(package, rf'install -D?m\d+ {re.escape(name)} ')
        self.assertIn('name=gelabber-native-linux-x64', script)
        self.assertIn('cd gelabber-native-linux-x64', package)


class Scripts(unittest.TestCase):
    def test_the_shell_scripts_parse(self):
        for script in ('packaging/native/tarball.sh', 'packaging/native/smoke.sh',
                       'packaging/arch/repo.sh', 'packaging/arch/update-pkgbuild.sh'):
            with self.subTest(script=script):
                subprocess.run(['bash', '-n', DESKTOP / script], check=True)

    def test_the_tarball_refuses_a_missing_core(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(['bash', NATIVE / 'tarball.sh', Path(tmp) / 'nothing.so', tmp],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('no media core', result.stderr)


if __name__ == '__main__':
    unittest.main()
