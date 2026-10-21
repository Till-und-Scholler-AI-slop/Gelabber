"""coturn-entry.sh has no default TURN password: static mode without
TURN_PASSWORD fails instead of starting with the old public one (#299)."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'coturn-entry.sh'

FAKE_TURNSERVER = """#!/bin/sh
printf '%s\\n' "$@" > "$TURNSERVER_ARGS"
"""


class CoturnEntry(unittest.TestCase):
    def start(self, **turn):
        """The script's exit code, its stderr and the arguments turnserver got
        (None if it was not started)."""
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / 'turnserver'
            fake.write_text(FAKE_TURNSERVER)
            fake.chmod(0o755)
            args = Path(tmp) / 'args'
            env = {name: value for name, value in os.environ.items()
                   if not name.startswith('TURN_')}
            env.update(turn, PATH=f'{tmp}:{os.environ["PATH"]}', TURNSERVER_ARGS=str(args))
            result = subprocess.run(['sh', str(SCRIPT)], env=env, capture_output=True,
                                    text=True, timeout=30)
            started = args.read_text().splitlines() if args.exists() else None
        return result.returncode, result.stderr, started

    def test_static_mode_without_password_fails(self):
        for turn in ({}, {'TURN_PASSWORD': ''}, {'TURN_AUTH_SECRET': '  '}):
            with self.subTest(**turn):
                code, stderr, started = self.start(**turn)
                self.assertEqual(code, 1)
                self.assertIn('TURN_PASSWORD is not set', stderr)
                self.assertIsNone(started)

    def test_static_mode_uses_the_password(self):
        code, stderr, started = self.start(TURN_PASSWORD='x')
        self.assertEqual(code, 0, stderr)
        self.assertIn('--lt-cred-mech', started)
        self.assertIn('--user=gelabber:x', started)
        self.assertNotIn('--use-auth-secret', started)

    def test_rest_mode_needs_no_password(self):
        code, stderr, started = self.start(TURN_AUTH_SECRET='private-secret')
        self.assertEqual(code, 0, stderr)
        self.assertIn('--use-auth-secret', started)
        self.assertIn('--static-auth-secret=private-secret', started)
        self.assertFalse([arg for arg in started if arg.startswith('--user')], started)

    def test_public_default_secret_is_refused(self):
        for turn in ({}, {'TURN_PASSWORD': 'x'}):
            with self.subTest(**turn):
                code, stderr, started = self.start(TURN_AUTH_SECRET='gelabberturnsecret', **turn)
                self.assertEqual(code, 1)
                self.assertIn('public default', stderr)
                self.assertIsNone(started)


if __name__ == '__main__':
    unittest.main()
