"""How desktop-release.yml attaches the app to a release: python3 -m unittest
discover -s desktop/packaging/tests

Both uploads wait for the whole build of desktop-native.yml, the third-party
notices included, so no release goes out with notices that are out of date,
and the pacman repository is published only after the tarball its PKGBUILD
downloads is attached. Read without a YAML library, which a runner's python
need not have."""
from pathlib import Path
import re
import unittest

WORKFLOWS = Path(__file__).parents[3] / '.github/workflows'
WORKFLOW = WORKFLOWS / 'desktop-release.yml'
BUILD = WORKFLOWS / 'desktop-native.yml'


def jobs(text):
    """Each job's `needs`, as a set, by job id."""
    body = text.split('\njobs:\n', 1)[1]
    found = {}
    current = None
    for line in body.splitlines():
        job = re.fullmatch(r'  ([A-Za-z0-9_-]+):\s*', line)
        if job:
            current = job.group(1)
            found[current] = set()
            continue
        needs = re.fullmatch(r'    needs:\s*(.+?)\s*', line)
        if current and needs:
            value = needs.group(1)
            names = value.strip('[]').split(',') if value.startswith('[') else [value]
            found[current] = {name.strip() for name in names if name.strip()}
    return found


class ReleaseWiring(unittest.TestCase):
    def setUp(self):
        self.jobs = jobs(WORKFLOW.read_text())

    def test_parser_reads_lists_and_single_names(self):
        sample = 'on: {}\njobs:\n  a:\n    runs-on: x\n  b:\n    needs: a\n  c:\n    needs: [a, b]\n'
        self.assertEqual(jobs(sample), {'a': set(), 'b': {'a'}, 'c': {'a', 'b'}})

    def test_the_build_holds_the_notices_and_both_packages(self):
        self.assertIn('    uses: ./.github/workflows/desktop-native.yml\n', WORKFLOW.read_text())
        build = jobs(BUILD.read_text())
        for job in ('notices', 'arch-repo', 'core-windows'):
            with self.subTest(job=job):
                self.assertIn(job, build)

    def test_uploads_wait_for_the_build(self):
        for job in ('release-asset', 'release-asset-windows'):
            with self.subTest(job=job):
                self.assertIn('build', self.jobs.get(job, set()))

    def test_the_repository_is_published_after_the_tarball(self):
        self.assertIn('release-asset', self.jobs.get('arch-repo-publish', set()))


def job_text(text, name):
    """The lines of one job, up to the next one."""
    body = text.split('\njobs:\n', 1)[1]
    found = re.search(rf'^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)', body, re.M | re.S)
    return found.group(1) if found else ''


class NativeWiring(unittest.TestCase):
    """The native app's assets go out under their own names; both update
    manifests name its installer, so installed Tauri apps update into it."""

    def setUp(self):
        self.release = WORKFLOW.read_text()
        self.build = BUILD.read_text()

    def test_the_build_makes_the_native_artifacts_outside_pull_requests(self):
        linux, windows = job_text(self.build, 'core-linux'), job_text(self.build, 'core-windows')
        self.assertIn('name: gelabber-native-linux-x64\n', linux)
        self.assertIn('desktop/packaging/native/tarball.sh', linux)
        self.assertIn('name: gelabber-native-windows-x64\n', windows)
        self.assertIn('desktop\\packaging\\native\\windows\\installer.nsi', windows)
        for job, step in ((linux, 'Package the native app'), (windows, 'Native installer')):
            with self.subTest(step=step):
                block = job.split(f'- name: {step}\n', 1)[1].split('\n      - name:', 1)[0]
                self.assertIn("github.event_name != 'pull_request'", block)

    def test_pull_requests_build_and_start_the_native_app(self):
        linux, windows = job_text(self.build, 'core-linux'), job_text(self.build, 'core-windows')
        self.assertIn('desktop/packaging/native/smoke.sh desktop/target/debug/gelabber', linux)
        self.assertIn('-p gelabber\n', windows)
        for job in (linux, windows):
            block = job.split('- name: Native app smoke test\n', 1)[1].split('\n      - name:', 1)[0]
            self.assertNotIn('pull_request', block)

    def test_the_release_uploads_the_native_assets_and_their_manifests(self):
        linux = job_text(self.release, 'release-asset')
        windows = job_text(self.release, 'release-asset-windows')
        self.assertIn('name: gelabber-native-linux-x64\n', linux)
        self.assertIn('gelabber-native-linux-x64.tar.gz', linux.split('gh release upload', 1)[1])
        self.assertIn('name: gelabber-native-windows-x64\n', windows)
        self.assertIn('setup=gelabber-native-windows-x64-setup.exe\n', windows)
        upload = windows.split('gh release upload', 1)[1]
        for asset in ('"$setup"', '"$setup.sha256"', '"$setup.sig"', 'latest-native.json', 'latest.json'):
            with self.subTest(asset=asset):
                self.assertRegex(upload, rf'(^|\s){re.escape(asset)}(\s|$)')
        # No Tauri app any more: no asset of its own.
        for job in (linux, windows):
            self.assertNotIn('gelabber-desktop-', job)

    def test_both_manifests_name_the_native_installer(self):
        # Installed Tauri apps (0.8) read latest.json: the same manifest.
        windows = job_text(self.release, 'release-asset-windows')
        self.assertIn('--rawfile signature "$setup.sig"', windows)
        self.assertIn('/releases/download/${TAG}/$setup"', windows)
        self.assertEqual(windows.count('> latest-native.json\n'), 1)
        self.assertIn('cp latest-native.json latest.json\n', windows)
        self.assertNotIn('> latest.json', windows)

    def test_the_native_app_reads_the_manifest_the_release_writes(self):
        update = (Path(__file__).parents[2] / 'ui/src/platform/update.rs').read_text()
        self.assertIn('pub const MANIFEST: &str = "latest-native.json";', update)
        endpoint = re.search(r'const ENDPOINT: &str =\s*"([^"]+)";', update).group(1)
        self.assertTrue(endpoint.endswith('/releases/latest/download/latest-native.json'), endpoint)
        self.assertIn('latest-native.json', job_text(self.release, 'release-asset-windows'))

    def test_the_pacman_repository_carries_the_native_package(self):
        arch = job_text(self.build, 'arch-repo')
        self.assertIn('name: gelabber-native-linux-x64\n', arch)
        self.assertNotIn('name: gelabber-desktop-linux-x64\n', arch)
        self.assertEqual(arch.count('repo.sh build "'), 2, 'test build and release rebuild')
        self.assertEqual(arch.count('gelabber-native-linux-x64.tar.gz site'), 2)
        # The upgrade test starts from the Tauri package of a release.
        test = arch.split('repo.sh test site', 1)[1].split('\n      - name:', 1)[0]
        self.assertIn('tauri/PKGBUILD tauri/gelabber-desktop-linux-x64.tar.gz "$TAURI_RELEASE"', test)
        self.assertRegex(arch, r'\n      TAURI_RELEASE: v0\.8\.\d+\n')


if __name__ == '__main__':
    unittest.main()
