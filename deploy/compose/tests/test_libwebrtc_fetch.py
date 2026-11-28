"""libwebrtc-fetch.sh takes a stored libwebrtc package only with the SHA-256
the repository lists (desktop/native/scripts/libwebrtc-fetch.sh, GEL-470).

The real script runs from a copy of desktop/native with its own list of
SHA-256s, and a stand-in gh on PATH that serves the release's asset from a
file (or holds none).
"""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

from prerequisites import requires

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / 'desktop/native/scripts/libwebrtc-fetch.sh'
KEY = 'libwebrtc-linux-x64-0123abcd'

# gh release download libwebrtc-packages --repo R --pattern NAME --dir DIR
FAKE_GH = r'''#!/usr/bin/env python3
import os, shutil, sys
args = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(' '.join(args) + '\n')
if args[:3] != ['release', 'download', 'libwebrtc-packages']:
    sys.exit(f'stand-in gh: unexpected call {args}')
asset = os.environ.get('ASSET', '')
if not asset:
    sys.exit('release asset not found')
pattern, directory = args[args.index('--pattern') + 1], args[args.index('--dir') + 1]
shutil.copy(asset, os.path.join(directory, pattern))
'''


@requires(shutil.which('zstd'), 'zstd')
class LibwebrtcFetch(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        native = self.tmp / 'desktop/native'
        (native / 'scripts').mkdir(parents=True)
        shutil.copy(SCRIPT, native / 'scripts')
        self.pins = native / 'libwebrtc-packages.sha256'
        bin_dir = self.tmp / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'gh').write_text(FAKE_GH)
        (bin_dir / 'gh').chmod(0o755)
        self.calls = self.tmp / 'calls'
        self.calls.touch()
        self.package = self.tmp / 'package'
        self.env = dict(os.environ, PATH=f'{bin_dir}:{os.environ["PATH"]}',
                        CALLS=str(self.calls), GITHUB_REPOSITORY='owner/repo', ASSET='')

    def asset(self, files):
        """A package archive holding `files` (path: content)."""
        content = self.tmp / 'content'
        for name, text in files.items():
            (content / name).parent.mkdir(parents=True, exist_ok=True)
            (content / name).write_text(text)
        tar = self.tmp / 'asset.tar'
        with tarfile.open(tar, 'w') as archive:
            for name in files:
                archive.add(content / name, arcname=name)
        subprocess.run(['zstd', '-q', '-f', str(tar), '-o', f'{tar}.zst'], check=True)
        return Path(f'{tar}.zst')

    def listed(self, sha256):
        self.pins.write_text(f'# SHA-256 of the stored packages\n{sha256}  {KEY}.tar.zst\n')

    def run_script(self, *args, asset=None):
        env = dict(self.env, ASSET=str(asset) if asset else '')
        return subprocess.run(['bash', str(self.tmp / 'desktop/native/scripts/libwebrtc-fetch.sh'),
                               *args, KEY, str(self.package)],
                              env=env, capture_output=True, text=True, timeout=60)

    def left_in_package(self):
        return sorted(str(p.relative_to(self.package)) for p in self.package.rglob('*')) \
            if self.package.exists() else []

    def test_the_listed_package_is_unpacked(self):
        asset = self.asset({'lib/libwebrtc.a': 'archive', 'include/api.h': 'header'})
        self.listed(hashlib.sha256(asset.read_bytes()).hexdigest())
        result = self.run_script(asset=asset)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'found=true\n')
        self.assertEqual((self.package / 'lib/libwebrtc.a').read_text(), 'archive')
        # The download is gone; only the package is left.
        self.assertEqual(self.left_in_package(), ['include', 'include/api.h', 'lib', 'lib/libwebrtc.a'])

    def test_a_package_with_another_sha256_is_refused(self):
        listed = self.asset({'lib/libwebrtc.a': 'the package that was stored'})
        self.listed(hashlib.sha256(listed.read_bytes()).hexdigest())
        replaced = self.asset({'lib/libwebrtc.a': 'a package someone put there since'})
        for args in ((), ('--or-build',)):
            with self.subTest(args=args):
                result = self.run_script(*args, asset=replaced)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('found=', result.stdout)
                self.assertIn('is not the one desktop/native/libwebrtc-packages.sha256 lists', result.stderr)
                self.assertEqual(self.left_in_package(), [])

    def test_a_package_without_the_library_is_refused(self):
        asset = self.asset({'include/api.h': 'header'})
        self.listed(hashlib.sha256(asset.read_bytes()).hexdigest())
        result = self.run_script('--or-build', asset=asset)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('found=', result.stdout)
        self.assertIn('holds no lib/libwebrtc.a', result.stderr)

    def test_an_unlisted_package_is_not_downloaded(self):
        asset = self.asset({'lib/libwebrtc.a': 'archive'})
        self.pins.write_text(f'{hashlib.sha256(asset.read_bytes()).hexdigest()}  {KEY}-other.tar.zst\n')
        result = self.run_script(asset=asset)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, '')
        self.assertIn(f'lists no {KEY}.tar.zst', result.stderr)
        result = self.run_script('--or-build', asset=asset)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'found=false\n')
        self.assertEqual(self.calls.read_text(), '')
        self.assertEqual(self.left_in_package(), [])

    def test_a_listed_package_missing_from_the_release(self):
        self.listed('0' * 64)
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, '')
        self.assertIn(f'holds no {KEY}.tar.zst', result.stderr)
        result = self.run_script('--or-build')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'found=false\n')
        self.assertEqual(self.left_in_package(), [])


if __name__ == '__main__':
    unittest.main()
