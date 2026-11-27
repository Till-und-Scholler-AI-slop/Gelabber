"""The app's packages against what the app says and what the Tauri app's
installs (up to 0.8) have: python3 -m unittest discover -s desktop/packaging/tests

The names only meet at run time (the desktop entry and the window's app id,
the installer and the Tauri app's folder and AppUserModelId, the pacman
package it upgrades), so a rename on one side is caught here rather than on
a user's desktop. makepkg is not needed; makensis compiles the installer
where it is installed (CI)."""
import importlib.util
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

# A missing tool skips a test here and fails it in CI, as everywhere (GEL-375).
_SPEC = importlib.util.spec_from_file_location(
    'prerequisites', Path(__file__).resolve().parents[3] / 'deploy/compose/tests/prerequisites.py')
prerequisites = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(prerequisites)

DESKTOP = Path(__file__).parents[2]
NATIVE = DESKTOP / 'packaging/native'
NSIS = NATIVE / 'windows/installer.nsi'
PKGBUILD = DESKTOP / 'packaging/arch/native/PKGBUILD'
ARCH = DESKTOP / 'packaging/arch'


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
        # The Tauri app's identifier, productName and publisher, which its
        # installs carry; the app keeps the identifier for its config folder.
        paths = (DESKTOP / 'ui/src/platform/paths.rs').read_text()
        identifier = re.search(r'pub const IDENTIFIER: &str = "([^"]+)";', paths).group(1)
        self.assertEqual(identifier, 'io.github.till-und-scholler-ai-slop.gelabber')
        self.assertEqual(self.defined['BUNDLEID'], identifier)
        self.assertEqual(self.defined['PRODUCTNAME'], 'Gelabber')
        self.assertEqual(self.defined['MANUFACTURER'], 'Gelabber')
        # Tauri's currentUser mode: the account's folder, no administrator.
        self.assertIn('InstallDir "$LOCALAPPDATA\\${PRODUCTNAME}"', self.text)
        self.assertIn('RequestExecutionLevel user', self.text)
        self.assertIn('Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${PRODUCTNAME}',
                      self.defined['UNINSTKEY'])
        self.assertIn('Delete "$INSTDIR\\${TAURIBINARY}"', self.text)
        self.assertEqual(self.defined['TAURIBINARY'], 'gelabber-desktop.exe')
        self.assertIn('!insertmacro CLOSE_ONE ${TAURIBINARY}', self.text)
        # The toasts' entry the app writes at every start goes with the app.
        self.assertEqual(self.defined['AUMIDKEY'], 'Software\\Classes\\AppUserModelId\\${BUNDLEID}')
        self.assertIn('DeleteRegKey HKCU "${AUMIDKEY}"', self.text)

    def test_it_accepts_the_tauri_updaters_arguments(self):
        # tauri-plugin-updater runs an NSIS setup with /P /UPDATE /R /ARGS.
        for option in ('/P', '/UPDATE', '/R', '/ARGS'):
            with self.subTest(option=option):
                self.assertIn(f'${{GetOptions}} $R0 "{option}" $R1', self.text)

    def test_it_installs_what_the_build_script_stages(self):
        files = re.findall(r'^\s+File "\$\{SOURCE\}\\([^"]+)"', self.text, re.M)
        files = [f.replace('${MAINBINARY}', self.defined['MAINBINARY']) for f in files]
        self.assertEqual(sorted(files), sorted([
            'gelabber.exe', 'gelabber_media.dll', 'icon.png', 'icon.ico', 'README.txt',
            'THIRD-PARTY-NOTICES.txt']))
        script = (NATIVE / 'windows/build-installer.ps1').read_text()
        for staged in ("'gelabber.exe'", "'gelabber_media.dll'", 'icon.ico', 'icon.png',
                       'THIRD-PARTY-NOTICES.txt', "'README.txt'"):
            with self.subTest(staged=staged):
                self.assertIn(staged, script)
        for removed in files + ['uninstall.exe', '${TAURIBINARY}']:
            name = removed if removed != self.defined['MAINBINARY'] else '${MAINBINARY}'
            with self.subTest(removed=removed):
                self.assertIn(f'Delete "$INSTDIR\\{name}"', self.text)

    def test_only_cmd_expands_environment_variables(self):
        # nsExec starts programs directly: a %VAR% reaches them as it is.
        for line in re.findall(r'^\s*nsExec::\w+ (.*)$', self.text, re.M):
            if re.search(r'%\w+%', line):
                with self.subTest(line=line):
                    self.assertIn('cmd.exe', line)

    def test_makensis_compiles_it(self):
        # The installer is built after merge only; this compile runs on pull requests.
        makensis = shutil.which('makensis') or prerequisites.missing('makensis (apt-get install nsis)')
        with tempfile.TemporaryDirectory() as tmp:
            stage = Path(tmp)
            for name in ('gelabber.exe', 'gelabber_media.dll', 'README.txt', 'THIRD-PARTY-NOTICES.txt'):
                (stage / name).write_text(name)
            for name in ('icon.ico', 'icon.png'):
                shutil.copy(DESKTOP / 'packaging/icons' / name, stage / name)
            # The release's name: makensis warns about a bare setup.exe.
            setup = stage / 'gelabber-native-windows-x64-setup.exe'
            subprocess.run([makensis, '-V2', '-WX', '-DVERSION=0.9.0-beta.1', '-DFILEVERSION=0.9.0.0',
                            f'-DSOURCE={stage}', f'-DOUTFILE={setup}', str(NSIS)], check=True)
            self.assertTrue(setup.is_file())


class ArchPackage(unittest.TestCase):
    def test_the_package_takes_over_the_tauri_packages_name(self):
        # Same name, newer version: pacman -Syu upgrades a Tauri install.
        self.assertEqual(bash_array(PKGBUILD, 'pkgname'), ['gelabber-desktop'])
        self.assertEqual(bash_array(PKGBUILD, 'replaces'), ['gelabber-desktop-native'])
        conflicts = bash_array(PKGBUILD, 'conflicts')
        for name in ('gelabber-desktop-native', 'gelabber-desktop-bin', 'gelabber-desktop-git'):
            with self.subTest(name=name):
                self.assertIn(name, conflicts)
        self.assertNotIn('gelabber-desktop', conflicts)
        # The Tauri package's command stays, next to the app's own.
        package = PKGBUILD.read_text().split('package() {', 1)[1]
        for command in ('gelabber', 'gelabber-desktop'):
            with self.subTest(command=command):
                self.assertIn(f'ln -s /opt/gelabber/gelabber "${{pkgdir}}/usr/bin/{command}"', package)
        update = (DESKTOP / 'ui/src/platform/update.rs').read_text()
        self.assertIn('const PACMAN_PACKAGE: &str = "gelabber-desktop";', update)

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


class ArchInstallScript(unittest.TestCase):
    """install.sh pins the key and address the README gives users; repo.sh
    test runs it against a test build's own key, so a typo here would only
    show on a user's machine."""

    def setUp(self):
        self.script = (ARCH / 'install.sh').read_text()
        self.readme = (ARCH / 'README.md').read_text()

    def setting(self, name):
        return re.search(rf'^{name}=(\S+)$', self.script, re.M).group(1)

    def test_it_pins_the_published_key(self):
        published = re.search(r'^Signing key fingerprint: `([0-9A-F]{40})`$', self.readme, re.M).group(1)
        self.assertEqual(self.setting('fingerprint'), published)
        self.assertIn(f'pacman-key --lsign-key {published}', self.readme)

    def test_it_adds_the_repository_the_readme_gives(self):
        server = re.search(r'^#    Server = (\S+)/\$arch$', self.readme, re.M).group(1)
        self.assertEqual(self.setting('repo_url'), server)
        self.assertIn(f'curl -fsSL {server}/install.sh | sudo sh', self.readme)
        self.assertIn('SigLevel = Required DatabaseRequired', self.script)
        self.assertIn('pacman -Syu --needed "$@" gelabber-desktop', self.script)

    def test_it_is_posix_sh(self):
        subprocess.run(['sh', '-n', ARCH / 'install.sh'], check=True)
        self.assertTrue(self.script.startswith('#!/bin/sh\n'))
        self.assertNotIn('local ', self.script)


class Scripts(unittest.TestCase):
    # That each shell script parses is shellcheck's (lint.sh, notices job).
    def test_the_tarball_refuses_a_missing_core(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(['bash', NATIVE / 'tarball.sh', Path(tmp) / 'nothing.so', tmp],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('no media core', result.stderr)



class CoreCheck(unittest.TestCase):
    """check-core.sh, which the Linux job runs on the core of every build and
    tarball.sh on the core it packages, on small libraries built here."""

    def library(self, source):
        compiler = shutil.which('cc')
        if not compiler or not shutil.which('nm') or not shutil.which('readelf'):
            prerequisites.missing('cc, nm or readelf')
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        # libc's puts, so that the library has a dependency to list, as the core has.
        (Path(tmp.name) / 'core.c').write_text('#include <stdio.h>\nint gm_log(void) { return puts("x"); }\n' + source)
        lib = Path(tmp.name) / 'libgelabber_media.so'
        subprocess.run([compiler, '-shared', '-fPIC', '-o', lib, Path(tmp.name) / 'core.c'], check=True)
        return lib

    def check(self, lib, *options):
        return subprocess.run(['bash', NATIVE / 'check-core.sh', *options, lib], capture_output=True, text=True)

    def test_a_core_that_exports_gm_only_passes_and_is_measured(self):
        lib = self.library('int gm_version(void) { return 1; }\n')
        with tempfile.NamedTemporaryFile('r') as summary:
            result = self.check(lib, '--summary', summary.name)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('| libgelabber_media.so (stripped) |', summary.read())
        self.assertIn('gm_version', result.stdout)

    def test_another_export_fails(self):
        result = self.check(self.library('int gm_version(void) { return 1; }\nint other(void) { return 2; }\n'))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('only gm_* symbols may be exported', result.stdout)

    def test_a_name_of_h264_code_fails(self):
        source = 'int gm_version(void) { return 1; }\n__attribute__((visibility("hidden"))) int WelsInitEncoderExt(void) { return 0; }\n'
        result = self.check(self.library(source))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('the core carries H264 code of OpenH264 or FFmpeg', result.stdout)



class VersionScript(unittest.TestCase):
    """version.sh, from which the workflows and the Windows scripts take the
    app's version (desktop-native.yml, release.yml)."""
    SCRIPT = NATIVE / 'version.sh'

    def version(self):
        text = (DESKTOP / 'Cargo.toml').read_text()
        return re.search(r'^\[workspace\.package\]\n(?:.*\n)*?version = "([^"]+)"', text, re.M).group(1)

    def run_script(self, *arguments):
        return subprocess.run(['bash', self.SCRIPT, *map(str, arguments)], capture_output=True, text=True,
                              env={'PATH': os.environ['PATH']})

    def test_it_reads_the_workspace_version(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, self.version() + '\n')

    def test_a_windows_checkout_reads_the_same(self):
        text = (DESKTOP / 'Cargo.toml').read_text()
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / 'Cargo.toml'
            manifest.write_bytes(text.replace('\n', '\r\n').encode())
            self.assertEqual(self.run_script(manifest).stdout, self.version() + '\n')

    def test_a_release_must_carry_its_tag(self):
        version = self.version()
        self.assertEqual(self.run_script('--tag', f'v{version}').returncode, 0)
        self.assertEqual(self.run_script('--tag', '').returncode, 0)
        for tag in (version, f'v{version}.1', 'v0.0.1'):
            with self.subTest(tag=tag):
                result = self.run_script('--tag', tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f'release {tag}: desktop/Cargo.toml, and with it the app, says {version}',
                              result.stderr)

    def test_the_windows_scripts_take_it_from_the_workflow(self):
        for path in sorted((NATIVE / 'windows').glob('*.ps1')):
            with self.subTest(script=path.name):
                self.assertNotRegex(path.read_text(), r'Cargo\.toml')


if __name__ == '__main__':
    unittest.main()
