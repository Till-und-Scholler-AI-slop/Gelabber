"""Compose passes only the variables it names: a key set in `.env` that no
compose file reads does nothing, as API_ALLOW_PLAIN_HTTP once did."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT.parents[1] / 'backend/api/src/config.rs'

# Read by Compose itself, or by the host's own Caddy (Caddyfile.homelab).
NOT_FOR_A_SERVICE = {'COMPOSE_FILE', 'COMPOSE_PROFILES', 'GELABBER_MINIO_DOMAIN'}
# Lets the API start without an object store; the stack always runs MinIO.
NOT_FORWARDED = {'API_ALLOW_MEMORY_STORE'}


def compose_text():
    return ''.join(line for path in sorted(ROOT.glob('compose*.yaml'))
                   for line in path.read_text().splitlines(True)
                   if not line.lstrip().startswith('#'))


def api_environment():
    """Keys of the api service's `environment` in compose.yaml."""
    block = re.search(r'^  api:\n(.*?)^  \S', (ROOT / 'compose.yaml').read_text(),
                      re.M | re.S).group(1)
    env = re.search(r'^    environment:\n((?:      .*\n|\s*\n)*)', block, re.M).group(1)
    return set(re.findall(r'^      ([A-Z][A-Z0-9_]*):', env, re.M))


class EnvKeys(unittest.TestCase):
    def test_every_example_key_reaches_compose(self):
        text = compose_text()
        for example in ('.env.example', '.env.homelab.example'):
            keys = set(re.findall(r'^#?([A-Z][A-Z0-9_]*)=', (ROOT / example).read_text(), re.M))
            self.assertTrue(keys)
            for key in sorted(keys - NOT_FOR_A_SERVICE):
                with self.subTest(example=example, key=key):
                    self.assertTrue(re.search(r'\$\{' + key + r'\b', text),
                                    f'{key} is set in {example} but no compose file reads it')

    def test_every_api_setting_reaches_the_api(self):
        settings = set(re.findall(r'^pub const ([A-Z][A-Z0-9_]*): &str = "\1";',
                                  CONFIG.read_text(), re.M))
        wanted = {key for key in settings if key.startswith('API_')} | {'RUST_LOG'}
        self.assertIn('API_ALLOW_PLAIN_HTTP', wanted)
        self.assertLessEqual(NOT_FORWARDED, settings)
        env = api_environment()
        self.assertEqual(sorted(wanted - NOT_FORWARDED - env), [])
        self.assertEqual(sorted(NOT_FORWARDED & env), [])


if __name__ == '__main__':
    unittest.main()
