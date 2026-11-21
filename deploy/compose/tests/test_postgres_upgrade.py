"""postgres-upgrade.sh stops nothing when there is nothing to upgrade: with
compose.yaml still on the old major, or the new cluster already in place.

A fake docker on PATH answers the script's questions; the full dump and
restore runs against real containers (deploy/README.md, "PostgreSQL major
upgrade").
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'postgres-upgrade.sh'

FAKE_DOCKER = """#!/bin/sh
printf '%s\\n' "$*" >> "$DOCKER_LOG"
case "$1 $2" in
  'compose config') printf '%s\\n' caddy:2 "$COMPOSE_POSTGRES" redis:8 ;;
  'compose ps') echo postgres-id ;;
  'inspect --format') echo gelabber_postgres_data ;;
  'run --rm') echo "$VOLUME_STATE" ;;
  *) ;;
esac
"""


class PostgresUpgrade(unittest.TestCase):
    def run_script(self, compose_postgres, volume_state):
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / 'docker'
            fake.write_text(FAKE_DOCKER)
            fake.chmod(0o755)
            log = Path(tmp) / 'log'
            env = dict(os.environ, PATH=f'{tmp}:{os.environ["PATH"]}', DOCKER_LOG=str(log),
                       COMPOSE_POSTGRES=compose_postgres, VOLUME_STATE=volume_state)
            result = subprocess.run(['sh', str(SCRIPT)], env=env, cwd=tmp, capture_output=True,
                                    text=True, timeout=30)
            calls = log.read_text().splitlines() if log.exists() else []
        return result, calls

    def test_same_major_stops_nothing(self):
        result, calls = self.run_script('postgres:18.6-alpine3.24', 'old')
        self.assertEqual(result.returncode, 1)
        self.assertIn('still names PostgreSQL 18', result.stderr)
        self.assertFalse([call for call in calls if ' stop ' in f' {call} '], calls)

    def test_upgraded_volume_is_left_alone(self):
        result, calls = self.run_script('postgres:19.0-alpine3.24', 'new')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Nothing to do', result.stdout)
        self.assertFalse([call for call in calls if ' stop ' in f' {call} '], calls)

    def test_volume_without_a_cluster_stops_nothing(self):
        result, calls = self.run_script('postgres:19.0-alpine3.24', 'none')
        self.assertEqual(result.returncode, 1)
        self.assertIn('holds no PostgreSQL 18 cluster', result.stderr)
        self.assertFalse([call for call in calls if ' stop ' in f' {call} '], calls)

    def test_default_old_image_is_the_major_before_compose(self):
        """A later major update has to move the script's default along."""
        compose = (ROOT / 'compose/compose.yaml').read_text()
        new = int(re.search(r'image: postgres:(\d+)\.', compose).group(1))
        old = int(re.search(r'GELABBER_POSTGRES_OLD_IMAGE:-postgres:(\d+)\.',
                            SCRIPT.read_text()).group(1))
        self.assertEqual(old, new - 1)


if __name__ == '__main__':
    unittest.main()
