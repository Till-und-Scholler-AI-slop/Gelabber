"""How desktop-release.yml attaches the app to a release: python3 -m unittest
discover -s desktop/packaging/tests

Both uploads wait for the whole build of desktop-native.yml, the third-party
notices included, so no release goes out with notices that are out of date,
and the pacman repository is published only after the tarball its PKGBUILD
downloads is attached. The workflows are read as parsed data, with
deploy/compose/tests/workflow_data.py and the PyYAML its requirements.txt pins."""
import importlib.util
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[3]


def _module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# A missing tool skips a test here and fails it in CI, as everywhere (GEL-375).
prerequisites = _module('prerequisites', ROOT / 'deploy/compose/tests/prerequisites.py')
try:
    workflow_data = _module('workflow_data', ROOT / 'deploy/compose/tests/workflow_data.py')
except ImportError:
    workflow_data = None

RELEASE = 'desktop-release.yml'
BUILD = 'desktop-native.yml'


def load(name):
    if workflow_data is None:
        prerequisites.missing('PyYAML (deploy/compose/tests/requirements.txt)')
    return workflow_data.load(name)


def job(name, job_id):
    return load(name)['jobs'][job_id]


def needs(name, job_id):
    value = job(name, job_id).get('needs', [])
    return {value} if isinstance(value, str) else set(value)


def scripts(step_owner):
    """The `run:` scripts of a job's steps, each ending in a newline."""
    return ''.join(f"{step['run'].rstrip()}\n" for step in step_owner.get('steps', []) if 'run' in step)


def artifacts(step_owner, action):
    """The artifact names of a job's upload- or download-artifact steps."""
    return [step['with']['name'] for step in step_owner.get('steps', [])
            if step.get('uses', '').startswith(f'actions/{action}-artifact@')]


def step(step_owner, name):
    found = [s for s in step_owner.get('steps', []) if s.get('name') == name]
    if len(found) != 1:
        raise AssertionError(f'{len(found)} steps named {name!r}')
    return found[0]


class ReleaseWiring(unittest.TestCase):
    def test_the_build_holds_the_notices_and_both_packages(self):
        self.assertEqual(job(RELEASE, 'build')['uses'], f'./.github/workflows/{BUILD}')
        for job_id in ('notices', 'arch-repo', 'core-windows'):
            with self.subTest(job=job_id):
                self.assertIn(job_id, load(BUILD)['jobs'])

    def test_uploads_wait_for_the_build(self):
        for job_id in ('release-asset', 'release-asset-windows'):
            with self.subTest(job=job_id):
                self.assertIn('build', needs(RELEASE, job_id))

    def test_the_repository_is_published_after_the_tarball(self):
        self.assertIn('release-asset', needs(RELEASE, 'arch-repo-publish'))
        self.assertIn('release-asset', needs(RELEASE, 'arch-repo-pages'))

    def test_the_pages_artifact_is_uploaded_a_job_before_the_deployment(self):
        # Right after an upload in the same job, deploy-pages often finds no
        # artifact (actions/deploy-pages#451, GEL-73).
        self.assertIn('arch-repo-pages', needs(RELEASE, 'arch-repo-publish'))
        uses = lambda job_id: [s.get('uses', '') for s in job(RELEASE, job_id)['steps']]
        self.assertTrue(any(u.startswith('actions/upload-pages-artifact@') for u in uses('arch-repo-pages')))
        self.assertFalse(any('upload-pages-artifact' in u for u in uses('arch-repo-publish')))
        self.assertIn('${{ needs.arch-repo-pages.outputs.artifact }}',
                      [s.get('with', {}).get('artifact_name') for s in job(RELEASE, 'arch-repo-publish')['steps']])

    def test_the_release_becomes_latest_only_once_both_apps_are_attached(self):
        # The apps' updater and the web's download links read releases/latest/download.
        self.assertEqual(needs(RELEASE, 'mark-latest'), {'release-asset', 'release-asset-windows'})
        mark = scripts(job(RELEASE, 'mark-latest'))
        self.assertIn('gh release edit "$TAG" --latest', mark)
        self.assertIn('sort -V', mark)
        # That image_set.py creates the release with --latest=false is
        # test_image_set.py's (deploy), which runs when image_set.py changes.


class NativeWiring(unittest.TestCase):
    """The native app's assets go out under their own names; the update
    manifest names its installer."""

    def test_the_build_makes_the_native_artifacts(self):
        linux, windows = job(BUILD, 'core-linux'), job(BUILD, 'core-windows')
        self.assertIn('gelabber-native-linux-x64', artifacts(linux, 'upload'))
        self.assertIn('desktop/packaging/native/tarball.sh', scripts(linux))
        self.assertIn('gelabber-native-windows-x64', artifacts(windows, 'upload'))
        self.assertEqual(step(windows, 'Native installer')['run'], './desktop/packaging/native/windows/build-installer.ps1')
        installer = (ROOT / 'desktop/packaging/native/windows/build-installer.ps1').read_text()
        self.assertIn('desktop\\packaging\\native\\windows\\installer.nsi', installer)
        # In which modes they are made (never for a pull request but a
        # release candidate): deploy/compose/tests/test_desktop_native_modes.py.

    def test_pull_requests_build_and_start_the_native_app(self):
        linux, windows = job(BUILD, 'core-linux'), job(BUILD, 'core-windows')
        self.assertIn('desktop/packaging/native/smoke.sh desktop/target/debug/gelabber',
                      step(linux, 'Native app smoke test')['run'])
        self.assertEqual(step(windows, 'Native app smoke test')['run'], './desktop/packaging/native/windows/smoke.ps1')
        self.assertRegex(scripts(windows), r'cargo build --release [^\n]*-p gelabber\n')
        for owner in (linux, windows):
            self.assertNotIn('pull_request', str(step(owner, 'Native app smoke test').get('if', '')))

    def test_the_release_uploads_the_native_assets_and_their_manifests(self):
        linux, windows = job(RELEASE, 'release-asset'), job(RELEASE, 'release-asset-windows')
        self.assertEqual(artifacts(linux, 'download'), ['gelabber-native-linux-x64'])
        self.assertIn('gelabber-native-linux-x64.tar.gz', scripts(linux).split('gh release upload', 1)[1])
        self.assertEqual(artifacts(windows, 'download'), ['gelabber-native-windows-x64'])
        self.assertIn('setup=gelabber-native-windows-x64-setup.exe\n', scripts(windows))
        upload = scripts(windows).split('gh release upload', 1)[1]
        for asset in ('"$setup"', '"$setup.sha256"', '"$setup.sig"', 'latest-native.json'):
            with self.subTest(asset=asset):
                self.assertRegex(upload, rf'(^|\s){re.escape(asset)}(\s|$)')
        # latest.json was the Tauri apps' manifest (0.8); none are left.
        self.assertNotIn('latest.json', upload.replace('latest-native.json', ''))
        # No Tauri app any more: no asset of its own.
        for owner in (linux, windows):
            self.assertNotIn('gelabber-desktop-', str(owner))

    def test_the_manifest_names_the_native_installer(self):
        windows = scripts(job(RELEASE, 'release-asset-windows'))
        self.assertIn('--rawfile signature "$setup.sig"', windows)
        self.assertIn('/releases/download/${TAG}/$setup"', windows)
        self.assertEqual(windows.count('> latest-native.json\n'), 1)

    def test_the_native_app_reads_the_manifest_the_release_writes(self):
        update = (ROOT / 'desktop/ui/src/platform/update.rs').read_text()
        self.assertIn('pub const MANIFEST: &str = "latest-native.json";', update)
        endpoint = re.search(r'const ENDPOINT: &str =\s*"([^"]+)";', update).group(1)
        self.assertTrue(endpoint.endswith('/releases/latest/download/latest-native.json'), endpoint)
        self.assertIn('latest-native.json', scripts(job(RELEASE, 'release-asset-windows')))

    def test_the_pacman_repository_carries_the_native_package(self):
        arch = job(BUILD, 'arch-repo')
        self.assertEqual(artifacts(arch, 'download'), ['gelabber-native-linux-x64'])
        runs = scripts(arch)
        self.assertEqual(runs.count('repo.sh build "'), 2, 'test build and release rebuild')
        self.assertEqual(runs.count('gelabber-native-linux-x64.tar.gz site'), 2)
        # A user's install from the repository, then upgrade and removal; no Tauri package any more.
        self.assertEqual(step(arch, 'Install, upgrade and remove like a user')['run'],
                         'desktop/packaging/arch/repo.sh test site gelabber-native-linux-x64.tar.gz')
        self.assertNotIn('tauri', str(arch).lower())



class LintPins(unittest.TestCase):
    def test_lint_sh_and_the_deploy_job_pin_the_same_shellcheck(self):
        # Two pins, one tool: a script that passes one shellcheck could fail the other.
        lint = (ROOT / 'desktop/packaging/lint.sh').read_text()
        pinned = dict(re.findall(r'^(SHELLCHECK_(?:VERSION|SHA256))=(\S+)$', lint, re.M))
        steps = [s for s in job('ci.yml', 'deploy')['steps'] if 'SHELLCHECK_VERSION' in s.get('env', {})]
        self.assertEqual(len(steps), 1)
        env = steps[0]['env']
        self.assertEqual(pinned, {'SHELLCHECK_VERSION': str(env['SHELLCHECK_VERSION']),
                                  'SHELLCHECK_SHA256': env['SHELLCHECK_SHA256']})


if __name__ == '__main__':
    unittest.main()
