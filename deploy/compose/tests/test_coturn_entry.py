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
                   if not name.startswith('TURN_') and name != 'MEDIA_ADVERTISED_IP'}
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

    def test_relays_only_to_public_peers_over_udp(self):
        """TURN users must not reach Redis, the API or the LAN through the relay."""
        for turn in ({'TURN_PASSWORD': 'x'}, {'TURN_AUTH_SECRET': 'private-secret'}):
            with self.subTest(**turn):
                code, stderr, started = self.start(**turn)
                self.assertEqual(code, 0, stderr)
                self.assertIn('--no-tcp-relay', started)
                self.assertIn('--no-multicast-peers', started)
                self.assertNotIn('--allow-loopback-peers', started)
                denied = {arg.split('=', 1)[1] for arg in started
                          if arg.startswith('--denied-peer-ip=')}
                for network in ('0.0.0.0-0.255.255.255',
                                '10.0.0.0-10.255.255.255',
                                '100.64.0.0-100.127.255.255',
                                '127.0.0.0-127.255.255.255',
                                '169.254.0.0-169.254.255.255',
                                '172.16.0.0-172.31.255.255',
                                '192.168.0.0-192.168.255.255',
                                '::1',
                                'fc00::-fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff',
                                'fe80::-febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff'):
                    self.assertIn(network, denied)
                self.assertFalse([arg for arg in started
                                  if arg.startswith('--allowed-peer-ip')], started)

    def test_a_private_sfu_address_stays_reachable(self):
        code, stderr, started = self.start(TURN_PASSWORD='x',
                                           MEDIA_ADVERTISED_IP='192.168.1.10')
        self.assertEqual(code, 0, stderr)
        self.assertIn('--allowed-peer-ip=192.168.1.10', started)
        self.assertIn('--denied-peer-ip=192.168.0.0-192.168.255.255', started)

    def test_a_loopback_sfu_address_is_not_allowed(self):
        for address in ('127.0.0.1', '::1', ' '):
            with self.subTest(address=address):
                code, stderr, started = self.start(TURN_PASSWORD='x',
                                                   MEDIA_ADVERTISED_IP=address)
                self.assertEqual(code, 0, stderr)
                self.assertFalse([arg for arg in started
                                  if arg.startswith('--allowed-peer-ip')], started)

    def test_operators_add_allowed_peers(self):
        code, stderr, started = self.start(
            TURN_PASSWORD='x', MEDIA_ADVERTISED_IP='192.168.1.10',
            TURN_ALLOWED_PEER_IPS=' 10.0.0.5, 172.20.0.1-172.20.0.9  fd00::7 ')
        self.assertEqual(code, 0, stderr)
        allowed = [arg for arg in started if arg.startswith('--allowed-peer-ip=')]
        self.assertEqual(allowed, ['--allowed-peer-ip=192.168.1.10',
                                   '--allowed-peer-ip=10.0.0.5',
                                   '--allowed-peer-ip=172.20.0.1-172.20.0.9',
                                   '--allowed-peer-ip=fd00::7'])

    def test_public_default_secret_is_refused(self):
        for turn in ({}, {'TURN_PASSWORD': 'x'}):
            with self.subTest(**turn):
                code, stderr, started = self.start(TURN_AUTH_SECRET='gelabberturnsecret', **turn)
                self.assertEqual(code, 1)
                self.assertIn('public default', stderr)
                self.assertIsNone(started)


if __name__ == '__main__':
    unittest.main()
