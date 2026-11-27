"""Compatibility code that is due goes before its release.

AGENTS.md ("Supported versions"): code kept for an older version names the
minor version that removes it, as `remove in vX.Y`. Once the workspace
version in backend/Cargo.toml reaches that minor version, the code is due,
and this test fails with its file and line. A release pull request bumps
that version and runs every CI area, so it stays red until the due code and
its tests are gone.
"""
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[3]
MARKER = re.compile(r'remove in v(\d+)\.(\d+)', re.IGNORECASE)
# The rule's own wording, the history of past releases and this test's
# examples are not markers.
NOT_MARKERS = (':(exclude)AGENTS.md', ':(exclude).claude/', ':(exclude).github/release-notes/',
               ':(exclude)deploy/compose/tests/test_compat_markers.py')


def workspace_minor(root):
    version = tomllib.loads((root / 'backend/Cargo.toml').read_text())['workspace']['package']['version']
    major, minor = version.split('.')[:2]
    return int(major), int(minor)


def due_markers(root, current):
    """`path:line: text` of every marker whose version is `current` or older,
    in tracked files and new files git does not ignore."""
    found = subprocess.run(
        ['git', 'grep', '--untracked', '-I', '-n', '-i', '-E', r'remove in v[0-9]+\.[0-9]+',
         '--', '.', *NOT_MARKERS],
        cwd=root, capture_output=True, text=True, check=False,
    )
    if found.returncode not in (0, 1):
        raise RuntimeError(found.stderr)
    due = []
    for line in found.stdout.splitlines():
        path, number, text = line.split(':', 2)
        for match in MARKER.finditer(text):
            if (int(match.group(1)), int(match.group(2))) <= current:
                due.append(f'{path}:{number}: {text.strip()}')
                break
    return due


class CompatMarkers(unittest.TestCase):
    def test_no_compatibility_code_is_due(self):
        current = workspace_minor(ROOT)
        due = due_markers(ROOT, current)
        self.assertEqual(
            due, [],
            f'due in v{current[0]}.{current[1]}: remove this code and its tests '
            'in a pull request of its own:\n' + '\n'.join(due))

    def test_a_marker_is_due_from_its_minor_version_on(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            (root / 'AGENTS.md').write_text('remove in v0.1\n')
            (root / 'a.rs').write_text('// Remove in V0.14: old clients.\n// remove in v0.15\n')
            (root / 'b.ts').write_text('x();\n// for v0.12 servers, remove in v0.13.\n')
            (root / 'c.py').write_text('# remove in v1.0\n')
            self.assertEqual(due_markers(root, (0, 14)), [
                'a.rs:1: // Remove in V0.14: old clients.',
                'b.ts:2: // for v0.12 servers, remove in v0.13.',
            ])
            self.assertEqual(due_markers(root, (0, 13)), ['b.ts:2: // for v0.12 servers, remove in v0.13.'])


if __name__ == '__main__':
    unittest.main()
