"""turn-check.sh flags an API and coturn started with different TURN values (#200)."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

FAKE_DOCKER = """#!/bin/sh
for last; do :; done
if [ "$1" = compose ]; then
  case "$last" in
    api) echo api-id ;;
    coturn) [ -n "$COTURN_ENV" ] && echo coturn-id ;;
  esac
  exit 0
fi
case "$3" in
  *State.Running*) [ "$last" = coturn-id ] && echo "${COTURN_RUNNING:-true}" || echo true; exit 0 ;;
esac
case "$last" in
  api-id) printf '%s\\n' $API_ENV ;;
  coturn-id) printf '%s\\n' $COTURN_ENV ;;
esac
"""


class TurnCheck(unittest.TestCase):
    def run_check(self, api_env, coturn_env, coturn_running='true'):
        with tempfile.TemporaryDirectory() as tmp:
            docker = Path(tmp) / 'docker'
            docker.write_text(FAKE_DOCKER)
            docker.chmod(0o755)
            env = dict(os.environ, PATH=f'{tmp}:{os.environ["PATH"]}',
                       API_ENV=api_env, COTURN_ENV=coturn_env,
                       COTURN_RUNNING=coturn_running)
            return subprocess.run(['sh', str(ROOT / 'turn-check.sh')], env=env,
                                  capture_output=True, text=True)

    def test_matching_values_pass(self):
        same = 'TURN_USERNAME=gelabber TURN_PASSWORD=neu TURN_AUTH_SECRET='
        result = self.run_check(same, f'HOME=/root {same}')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_stale_coturn_fails_without_leaking_values(self):
        result = self.run_check(
            'TURN_USERNAME=gelabber TURN_PASSWORD=neu-geheim',
            'TURN_USERNAME=gelabber TURN_PASSWORD=gelabberturn')
        self.assertEqual(result.returncode, 1)
        self.assertIn('TURN_PASSWORD', result.stderr)
        self.assertNotIn('TURN_USERNAME', result.stderr)
        self.assertIn('--force-recreate coturn api', result.stderr)
        self.assertNotIn('neu-geheim', result.stderr + result.stdout)
        self.assertNotIn('gelabberturn', result.stderr + result.stdout)

    def test_stopped_bundled_coturn_fails(self):
        same = 'TURN_USERNAME=gelabber TURN_PASSWORD=neu'
        result = self.run_check(same, same, coturn_running='false')
        self.assertEqual(result.returncode, 1)
        self.assertIn('coturn läuft nicht', result.stderr)

    def test_without_bundled_coturn_only_hints(self):
        result = self.run_check('TURN_PASSWORD=x', '')
        self.assertEqual(result.returncode, 0)
        self.assertIn('externen TURN', result.stdout)


if __name__ == '__main__':
    unittest.main()
