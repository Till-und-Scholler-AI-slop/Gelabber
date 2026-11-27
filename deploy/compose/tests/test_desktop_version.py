"""The desktop workspace's version against the server's.

release.yml refuses a release whose tag is not desktop/Cargo.toml's version,
but only after the images are promoted. This holds the rule on every change
to either Cargo.toml instead: the desktop app has the backend workspace's
version, or is a pre-release of a later one (an alpha cut from a feature
branch while the server is still on the released line).
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest

from workflow_data import load, names, scripts
from workflow_expressions import evaluate

ROOT = Path(__file__).resolve().parents[3]
SEMVER = re.compile(
    r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)'
    r'(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?'
    r'(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?')


def parse(version):
    match = SEMVER.fullmatch(version)
    if not match:
        raise ValueError(f'{version!r} is not a semver version')
    return tuple(int(part) for part in match.group(1, 2, 3)), match.group(4)


def desktop_version_allowed(desktop, root):
    """Equal to the root version, or a pre-release of a greater version."""
    if desktop == root:
        return True
    core, pre = parse(desktop)
    root_core, root_pre = parse(root)
    return pre is not None and root_pre is None and core > root_core


def workspace_version(path):
    return tomllib.loads(path.read_text())['workspace']['package']['version']


class DesktopVersion(unittest.TestCase):
    def test_desktop_version_fits_the_server_version(self):
        desktop = workspace_version(ROOT / 'desktop/Cargo.toml')
        root = workspace_version(ROOT / 'backend/Cargo.toml')
        self.assertTrue(
            desktop_version_allowed(desktop, root),
            f'desktop/Cargo.toml says {desktop}, backend/Cargo.toml {root}: the desktop app has the '
            f'server\'s version, or a pre-release (with a "-") of a later one',
        )

    def test_rule(self):
        allowed = [('0.5.2', '0.5.2'), ('0.6.0-alpha.1', '0.5.2'), ('0.5.3-rc.1', '0.5.2'),
                   ('1.0.0-beta', '0.9.9')]
        refused = [('0.6.0', '0.5.2'), ('0.5.1', '0.5.2'), ('0.5.2-alpha.1', '0.5.2'),
                   ('0.5.1-alpha.1', '0.5.2'), ('0.6.0-alpha.1', '0.6.0-alpha.1-x'),
                   ('0.6.0+build', '0.5.2')]
        for desktop, root in allowed:
            with self.subTest(desktop=desktop, root=root):
                self.assertTrue(desktop_version_allowed(desktop, root))
        for desktop, root in refused:
            with self.subTest(desktop=desktop, root=root):
                self.assertFalse(desktop_version_allowed(desktop, root))
        with self.assertRaises(ValueError):
            desktop_version_allowed('0.6', '0.5.2')


class VersionReaders(unittest.TestCase):
    def test_no_workflow_reads_the_version_by_itself(self):
        # desktop/packaging/native/version.sh does (its tests: desktop/packaging/tests).
        parses = re.compile(r'\^version = |Cargo\.toml.*version|version.*Cargo\.toml')
        found = [f'{name}: {job}: {step}' for name in names() for job, step, script in scripts(load(name))
                 if re.search(r'desktop[\\/]Cargo\.toml', script) and parses.search(script)]
        self.assertEqual(found, [])


VERSION_SH = 'desktop/packaging/native/version.sh'
# The steps that read the version and, in a release, check it against the tag.
RELEASE_CHECKS = {
    ('release.yml', 'github-release', "Desktop app has the release's version"),
    ('desktop-native.yml', 'core-windows', 'Version and build settings'),
    ('desktop-native.yml', 'prebuilt', 'Sign the native update'),
    ('desktop-native.yml', 'arch-repo', 'Build package and repository'),
}
STUB = '#!/bin/sh\nmkdir -p site\n'


class ReleaseTagChecks(unittest.TestCase):
    """Each step that reads the version runs, as the release workflows set
    it up, with the release's tag: it must stop a release whose tag is not
    desktop/Cargo.toml's version. The step's `run:` comes from the parsed
    workflow and its env from the release tag; what follows the check
    (signing, repo.sh) is stubbed."""

    def sites(self):
        return {(name, job_id, step['name']): (load(name), job, step)
                for name in names() for job_id, job in load(name).get('jobs', {}).items()
                for step in job.get('steps', []) if VERSION_SH in str(step.get('run', ''))}

    def environment(self, workflow, job, step, tag):
        context = {'inputs.release_tag': tag, 'github.event.release.tag_name': tag,
                   'needs.decide.outputs.tag': tag, 'github.ref': f'refs/tags/{tag}', 'github.token': 'token'}
        env = {}

        def lookup(path):
            if path in context:
                return context[path]
            if path.startswith('env.'):
                return env.get(path[4:], '')
            if path.startswith('secrets.'):
                return 'secret'
            if path.startswith(('vars.', 'inputs.')):
                return ''
            raise ValueError(f'{step["name"]}: no value for {path}')

        for scope in (workflow.get('env') or {}, job.get('env') or {}, step.get('env') or {}):
            for key, value in scope.items():
                value = evaluate(value, lookup) if isinstance(value, str) and '${{' in value else value
                env[key] = {True: 'true', False: 'false', None: ''}.get(value, value) \
                    if isinstance(value, (bool, type(None))) else str(value)
        return env

    def run_step(self, site, tag, version='0.14.0'):
        workflow, job, step = self.sites()[site]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            files = {'desktop/Cargo.toml': f'[workspace.package]\nversion = "{version}"\n',
                     VERSION_SH: (ROOT / VERSION_SH).read_text(),
                     'desktop/native/windows-build.env': (ROOT / 'desktop/native/windows-build.env').read_text(),
                     'desktop/packaging/arch/repo.sh': STUB, 'desktop/packaging/arch/README.md': '',
                     'bin/npx': '#!/bin/sh\n', 'bin/sha256sum': '#!/bin/sh\n'}
            for name, text in files.items():
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text(text)
                (root / name).chmod(0o755)
            (root / 'windows').mkdir()
            env = {**self.environment(workflow, job, step, tag),
                   'PATH': f'{root / "bin"}:{os.environ["PATH"]}', 'GITHUB_ACTIONS': 'true',
                   'GITHUB_ENV': str(root / 'github_env'), 'GITHUB_OUTPUT': str(root / 'github_output'),
                   'GITHUB_REF': f'refs/tags/{tag}', 'GITHUB_RUN_NUMBER': '7', 'GITHUB_REPOSITORY': 'owner/repo'}
            options = ['--noprofile', '--norc', '-eo', 'pipefail'] if step.get('shell') == 'bash' else ['-e']
            result = subprocess.run(['bash', *options, '-c', str(step['run'])], cwd=root, env=env,
                                    capture_output=True, text=True)
            written = (root / 'github_env').read_text() if (root / 'github_env').exists() else ''
            return result, written

    def test_the_steps_that_read_the_version(self):
        self.assertEqual(set(self.sites()), RELEASE_CHECKS)

    def test_a_release_whose_tag_is_the_version_passes(self):
        for site in RELEASE_CHECKS:
            with self.subTest(site=site):
                result, _ = self.run_step(site, 'v0.14.0')
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_release_whose_tag_is_another_version_stops(self):
        for site in RELEASE_CHECKS:
            with self.subTest(site=site):
                result, _ = self.run_step(site, 'v0.14.1')
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('release v0.14.1: desktop/Cargo.toml, and with it the app, says 0.14.0',
                              result.stdout + result.stderr)

    def test_the_windows_job_hands_the_version_on(self):
        site = ('desktop-native.yml', 'core-windows', 'Version and build settings')
        result, written = self.run_step(site, '', version='0.15.0-rc.1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('DESKTOP_VERSION=0.15.0-rc.1\n', written)
        self.assertIn('RUSTFLAGS=-C target-feature=+crt-static\nMESON_ARGS=--vsenv -Db_vscrt=mt\n', written)

if __name__ == '__main__':
    unittest.main()
