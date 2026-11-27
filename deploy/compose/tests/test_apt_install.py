"""apt-install.sh installs Ubuntu packages on the runners (GEL-404).

The real script runs with fake sudo, timeout and apt-get on PATH and a
temporary /etc/apt (APT_INSTALL_ETC). The fake apt-get reads the sources apt
would read, honouring Dir::Etc::SourceList and Dir::Etc::SourceParts, and
fails an update when one of them is in BROKEN, as apt does.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[3] / '.github/workflows/scripts/apt-install.sh'

FAKE_APT_GET = r'''#!/usr/bin/env python3
import os, sys
from pathlib import Path

etc = Path(os.environ['APT_INSTALL_ETC'])
broken = os.environ.get('BROKEN', '').split()
missing = os.environ.get('MISSING', '').split()
options, rest = {}, []
args = iter(sys.argv[1:])
for arg in args:
    if arg == '-o':
        key, _, value = next(args).partition('=')
        options[key.lower()] = value
    else:
        rest.append(arg)
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(' '.join(rest) + '\n')

def first_mirror(path):
    return Path(path).read_text().split()[0]

def uris():
    main = Path(options.get('dir::etc::sourcelist', etc / 'sources.list'))
    parts = Path(options.get('dir::etc::sourceparts', etc / 'sources.list.d'))
    files = [main] if main.is_file() else []
    if parts.is_dir():
        files += sorted(f for f in parts.iterdir() if f.suffix in ('.list', '.sources'))
    for f in files:
        for word in f.read_text().split():
            if word.startswith('mirror+file:'):
                yield first_mirror(word[len('mirror+file:'):])
            elif word.startswith(('http://', 'https://')):
                yield word

if rest[0] == 'update':
    failed = [u for u in uris() if u in broken]
    for u in failed:
        print(f'E: Failed to fetch {u}InRelease  403  Forbidden')
    sys.exit(100 if failed else 0)
if rest[0] == 'install':
    unknown = [p for p in rest[1:] if not p.startswith('-') and p in missing]
    for p in unknown:
        print(f'E: Unable to locate package {p}')
    if unknown:
        sys.exit(100)
    if '--download-only' in rest and first_mirror(etc / 'apt-mirrors.txt') in broken:
        print('E: Failed to fetch packages')
        sys.exit(100)
    sys.exit(0)
sys.exit(f'fake apt-get: unexpected call {rest}')
'''

FAKE_SUDO = '#!/bin/sh\nexec "$@"\n'
# The script calls: timeout --kill-after=10 SECONDS COMMAND...
FAKE_TIMEOUT = '#!/bin/sh\nshift 2\nexec "$@"\n'

AZURE = 'http://azure.archive.ubuntu.com/ubuntu/'
ARCHIVE = 'https://archive.ubuntu.com/ubuntu/'
MICROSOFT = 'https://packages.microsoft.com/ubuntu/24.04/prod/'


class AptInstall(unittest.TestCase):
    def run_script(self, *packages, layout='deb822', broken=(), missing=()):
        """Runs the script; returns (result, apt-get calls, mirror list after)."""
        tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        etc, bin_dir = tmp / 'etc-apt', tmp / 'bin'
        (etc / 'sources.list.d').mkdir(parents=True)
        bin_dir.mkdir()
        mirrors = etc / 'apt-mirrors.txt'
        mirrors.write_text(f'{AZURE}\tpriority:1\n{ARCHIVE}\tpriority:2\n')
        ubuntu = f'mirror+file:{mirrors}'
        # The runner images' layouts: deb822 on 24.04 and 26.04, classic on 22.04.
        if layout == 'deb822':
            (etc / 'sources.list').write_text('# Ubuntu sources have moved to ubuntu.sources\n')
            (etc / 'sources.list.d/ubuntu.sources').write_text(
                f'Types: deb\nURIs: {ubuntu}\nSuites: noble noble-updates\nComponents: main\n')
        else:
            (etc / 'sources.list').write_text(f'deb {ubuntu} jammy main\n')
        (etc / 'sources.list.d/microsoft-prod.list').write_text(f'deb [arch=amd64] {MICROSOFT} noble main\n')
        for name, text in (('apt-get', FAKE_APT_GET), ('sudo', FAKE_SUDO), ('timeout', FAKE_TIMEOUT)):
            (bin_dir / name).write_text(text)
            (bin_dir / name).chmod(0o755)
        calls = tmp / 'calls'
        calls.touch()
        env = dict(os.environ, PATH=f'{bin_dir}:{os.environ["PATH"]}', APT_INSTALL_ETC=str(etc),
                   RUNNER_TEMP=str(tmp), CALLS=str(calls), BROKEN=' '.join(broken),
                   MISSING=' '.join(missing))
        result = subprocess.run(['bash', str(SCRIPT), *packages], env=env,
                                capture_output=True, text=True, timeout=60)
        return result, calls.read_text().splitlines(), mirrors.read_text().split()[0]

    def test_a_failing_third_party_repository_does_not_fail_the_install(self):
        for layout in ('deb822', 'classic'):
            with self.subTest(layout=layout):
                result, calls, first = self.run_script('nsis', layout=layout, broken=[MICROSOFT])
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual([c for c in calls if c == 'update'], ['update'])
                self.assertIn('install -y --no-install-recommends --no-download nsis', calls)
                self.assertNotIn('switching', result.stdout)
                self.assertEqual(first, AZURE)

    def test_a_failing_first_mirror_leads_to_the_next(self):
        result, calls, first = self.run_script('nsis', broken=[AZURE, MICROSOFT])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f'switching to {ARCHIVE}', result.stdout)
        self.assertEqual(first, ARCHIVE)
        self.assertIn('install -y --no-install-recommends --no-download nsis', calls)

    def test_a_missing_package_fails(self):
        result, calls, _ = self.run_script('nsis', 'no-such-package', broken=[MICROSOFT],
                                           missing=['no-such-package'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Unable to locate package no-such-package', result.stdout)
        self.assertFalse([c for c in calls if '--no-download' in c], calls)


if __name__ == '__main__':
    unittest.main()
