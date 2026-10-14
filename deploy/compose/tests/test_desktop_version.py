"""The desktop workspace's version against the server's.

release.yml refuses a release whose tag is not desktop/Cargo.toml's version,
but only after the images are promoted. This holds the rule on every change
to either Cargo.toml instead: the desktop app has the root workspace's
version, or is a pre-release of a later one (an alpha cut from a feature
branch while the server is still on the released line).
"""
from pathlib import Path
import re
import tomllib
import unittest

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
        root = workspace_version(ROOT / 'Cargo.toml')
        self.assertTrue(
            desktop_version_allowed(desktop, root),
            f'desktop/Cargo.toml says {desktop}, Cargo.toml {root}: the desktop app has the '
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

    def test_ci_runs_this_when_either_version_changes(self):
        ci = (ROOT / '.github/workflows/ci.yml').read_text()
        paths = ci.split('pull_request:', 1)[1].split('push:', 1)[0]
        for path in ('Cargo.toml', 'desktop/Cargo.toml', 'deploy/compose/**'):
            with self.subTest(path=path):
                self.assertIn(f'      - {path}\n', paths)
        self.assertIn('python3 -m unittest discover -s deploy/compose/tests', ci)


if __name__ == '__main__':
    unittest.main()
