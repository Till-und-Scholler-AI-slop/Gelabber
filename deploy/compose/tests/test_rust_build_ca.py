"""rust-build-ca.sh trusts the chain crates.io presents only when the build
asks for it: a TLS-inspecting proxy is opt-in, never a silent fallback."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / 'backend/build/rust-build-ca.sh'
ARG = 'TRUST_PRESENTED_CRATES_CA'

# Stand-ins for the image's tools. The verifying s_client fails; -showcerts
# (capturing the chain) would succeed, so a refusal is the script's own doing.
FAKES = {
    'apt-get': 'exit 0',
    'update-ca-certificates': 'echo update-ca-certificates >> "$CALLS"',
    'openssl': 'echo "openssl $*" >> "$CALLS"\n'
               'case "$*" in *-verify_return_error*) exit 1;; esac\n'
               'echo "-----BEGIN CERTIFICATE-----"; echo "-----END CERTIFICATE-----"',
}


class RustBuildCa(unittest.TestCase):
    def test_failed_verification_stops_the_build_without_the_build_arg(self):
        for value in (None, '', 'false', 'yes'):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as tmp:
                for name, body in FAKES.items():
                    fake = Path(tmp) / name
                    fake.write_text('#!/bin/sh\n' + body + '\n')
                    fake.chmod(0o755)
                calls = Path(tmp) / 'calls'
                env = {k: v for k, v in os.environ.items() if k != ARG}
                env.update(PATH=f'{tmp}:{os.environ["PATH"]}', CALLS=str(calls),
                           CARGO_HOME=str(Path(tmp) / 'cargo'))
                if value is not None:
                    env[ARG] = value
                result = subprocess.run(['sh', str(SCRIPT)], env=env, capture_output=True,
                                        text=True, timeout=30)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f'--build-arg {ARG}=true', result.stderr)
                log = calls.read_text().splitlines()
                self.assertFalse([line for line in log if '-showcerts' in line], log)
                self.assertEqual(log.count('update-ca-certificates'), 1)
                self.assertFalse((Path(tmp) / 'cargo').exists())

    def test_the_build_arg_defaults_to_off_and_ci_never_sets_it(self):
        for dockerfile in ('backend/api/Dockerfile', 'backend/media/Dockerfile'):
            with self.subTest(dockerfile=dockerfile):
                text = (ROOT / dockerfile).read_text()
                base = text.split('FROM toolchain AS base\n', 1)[1].split('\nFROM ', 1)[0]
                self.assertRegex(base, rf'(?m)^ARG {ARG}=false\n(.*\n)*RUN sh /tmp/rust-build-ca\.sh')
        for workflow in (ROOT / '.github/workflows').glob('*.yml'):
            self.assertNotIn(ARG, workflow.read_text(), workflow.name)


if __name__ == '__main__':
    unittest.main()
