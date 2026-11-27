"""The update signature check of a candidate's installer (.github/workflows/scripts/update_signature.py).

The fixture was signed with `npx @tauri-apps/cli@2.12.3 signer sign
--app-version 0.14.0` (the pinned CLI desktop-native.yml signs with) and a
throwaway key whose public half is test-updater.pub.
"""
import importlib.util
from pathlib import Path
import unittest

from prerequisites import requires

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/update_signature.py'
FIXTURES = Path(__file__).resolve().parent / 'fixtures/update-signature'

try:
    spec = importlib.util.spec_from_file_location('update_signature', SCRIPT)
    update_signature = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(update_signature)
except ModuleNotFoundError:  # cryptography; desktop-native.yml installs python3-cryptography
    update_signature = None


@requires(update_signature is not None, 'the cryptography package')
class VerifyTest(unittest.TestCase):
    def setUp(self):
        self.data = (FIXTURES / 'installer.exe').read_bytes()
        self.signature = (FIXTURES / 'installer.exe.sig').read_text()
        self.key = (FIXTURES / 'test-updater.pub').read_text()

    def test_the_tauri_signature_for_its_version(self):
        trusted = update_signature.verify(self.data, self.signature, self.key, '0.14.0')
        self.assertIn('version:0.14.0', trusted)
        update_signature.verify(self.data, self.signature, self.key, 'v0.14.0')

    def test_another_version_fails(self):
        with self.assertRaisesRegex(ValueError, 'signed for 0.14.0, not 0.14.1'):
            update_signature.verify(self.data, self.signature, self.key, '0.14.1')

    def test_a_changed_file_fails(self):
        with self.assertRaisesRegex(ValueError, 'does not match'):
            update_signature.verify(self.data + b'x', self.signature, self.key, '0.14.0')

    def test_the_releases_key_rejects_a_signature_by_another(self):
        releases = (ROOT / 'desktop/packaging/updater.pub').read_text()
        with self.assertRaisesRegex(ValueError, 'another key'):
            update_signature.verify(self.data, self.signature, releases, '0.14.0')


if __name__ == '__main__':
    unittest.main()
