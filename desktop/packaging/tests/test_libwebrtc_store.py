"""desktop/native/scripts/libwebrtc-stored.sh, which tells desktop-native.yml
whether a libwebrtc package is missing from the libwebrtc-packages release
(missing=true hands it to libwebrtc-store.yml), with a stub `gh`:
python3 -m unittest discover -s desktop/packaging/tests"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).parents[2] / 'native/scripts/libwebrtc-stored.sh'
ASSET = 'libwebrtc-linux-x64-0123abcd.tar.zst'


class Stored(unittest.TestCase):
    def run_script(self, gh):
        with tempfile.TemporaryDirectory() as tmp:
            stub = Path(tmp) / 'gh'
            stub.write_text('#!/bin/sh\n' + gh)
            stub.chmod(0o755)
            env = {**os.environ, 'PATH': f'{tmp}:{os.environ["PATH"]}', 'GITHUB_REPOSITORY': 'owner/repo'}
            return subprocess.run(['bash', SCRIPT, ASSET], env=env, capture_output=True, text=True)

    def test_a_package_the_release_lacks_is_missing(self):
        result = self.run_script('printf "%s\\n" other.tar.zst libwebrtc-linux-x64-0123abcd.tar.zst.sig\n')
        self.assertEqual((result.returncode, result.stdout), (0, 'missing=true\n'), result.stderr)

    def test_a_package_the_release_holds_is_not(self):
        result = self.run_script(f'printf "%s\\n" other.tar.zst {ASSET}\n')
        self.assertEqual((result.returncode, result.stdout), (0, 'missing=false\n'), result.stderr)

    def test_without_the_release_nothing_is_stored_and_it_says_so(self):
        result = self.run_script('echo "release not found" >&2; exit 1\n')
        self.assertEqual((result.returncode, result.stdout), (0, 'missing=false\n'))
        self.assertIn('::warning::The libwebrtc-packages release is missing', result.stderr)

    def test_it_asks_for_the_assets_of_that_release(self):
        result = self.run_script('echo "$@" >&2\n')
        self.assertIn('release view libwebrtc-packages --repo owner/repo --json assets', result.stderr)


if __name__ == '__main__':
    unittest.main()
