"""The public pages (#522) reach the web container: compose passes every value
the web image's start script reads and mounts the operator's texts, those
texts stay out of commits, and both Caddyfiles hand the pages' paths to web."""
import fnmatch
import json
from pathlib import Path
import re
import shutil
import subprocess
import unittest

import yaml

from compose_config import config
from test_homelab_caddy import _adapt, _find_caddy

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
SCRIPT = REPO / 'web/docker-entrypoint.d/50-gelabber-public-pages.sh'
# Overrides of the script's paths for its unit test, not operator settings.
SCRIPT_TEST_ONLY = {'GELABBER_PUBLIC_TEMPLATES', 'GELABBER_PUBLIC_OUT',
                    'GELABBER_LEGAL_DIR', 'GELABBER_LANDING_MAP'}
SETTINGS = {
    'GELABBER_LANDING', 'GELABBER_OPERATOR_NAME', 'GELABBER_OPERATOR_STREET',
    'GELABBER_OPERATOR_CITY', 'GELABBER_OPERATOR_EMAIL', 'GELABBER_OPERATOR_PHONE',
    'GELABBER_CONTENT_RESPONSIBLE', 'GELABBER_REPORT_EMAIL', 'GELABBER_CONTACT_EMAIL',
    'GELABBER_INVITE_REQUEST', 'GELABBER_SOURCE_URL', 'GELABBER_OPERATOR_GUIDE_URL',
    'APP_PUBLIC_URL',
}
# Derived by Compose from the API's keys (#542): only whether they are set,
# never the addresses.
DERIVED = {'GELABBER_ACCOUNT_MAIL': '${SMTP_HOST:+on}',
           'GELABBER_FEEDBACK_OPERATORS': '${API_OPERATOR_EMAILS:+on}'}
PUBLIC_PATHS = ('/', '/de/', '/en/', '/de/impressum', '/en/privacy', '/robots.txt',
                '/sitemap.xml', '/public-pages.json', '/de/feedback',
                '/en/feedback/thanks')
# The upstream that serves web, and the site's host, by Caddyfile.
WEB_UPSTREAMS = {'Caddyfile': ('web:80', None), 'Caddyfile.homelab': ('127.0.0.1:8088', 'gelabber.home.arpa')}


def route(adapted, path, host):
    """What Caddy's adapted configuration does with a request: the dial of the
    reverse_proxy that answers it, or the handler that does."""
    def walk(routes):
        for entry in routes:
            if not any(matches(matcher) for matcher in entry.get('match') or [{}]):
                continue
            for handler in entry.get('handle', []):
                if handler['handler'] == 'subroute':
                    found = walk(handler['routes'])
                    if found:
                        return found
                elif handler['handler'] == 'reverse_proxy':
                    return handler['upstreams'][0]['dial']
                elif handler['handler'] in ('static_response', 'file_server'):
                    return handler['handler']
        return None

    def matches(matcher):
        hosts, paths = matcher.get('host'), matcher.get('path')
        return ((hosts is None or host in hosts)
                and (paths is None or any(fnmatch.fnmatchcase(path, pattern) for pattern in paths)))

    for server in adapted['apps']['http']['servers'].values():
        found = walk(server['routes'])
        if found:
            return found
    return None


def web():
    return config()['services']['web']


@unittest.skipUnless(shutil.which('docker'), 'Docker CLI is unavailable')
class Compose(unittest.TestCase):
    def test_web_receives_every_setting_and_nothing_when_unset(self):
        unset, given = web()['environment'], config({key: f'set-{key}' for key in SETTINGS})
        for key in sorted(SETTINGS):
            with self.subTest(key=key):
                self.assertEqual(unset.get(key), '')
                self.assertEqual(given['services']['web']['environment'].get(key), f'set-{key}')

    def test_web_learns_only_whether_mail_and_operators_are_on(self):
        # Every mail and operator setting the API reads, filled in: none of
        # their values may reach web.
        api = json.dumps(yaml.safe_load((ROOT / 'compose.yaml').read_text())['services']['api']['environment'])
        secret = {key: f'{key.lower()}.example.test' for key in set(re.findall(r'\$\{(\w+)', api))
                  if key.startswith('SMTP_') or key == 'API_OPERATOR_EMAILS'}
        self.assertLessEqual({'SMTP_HOST', 'SMTP_PASSWORD', 'API_OPERATOR_EMAILS'}, set(secret))
        unset, given = web()['environment'], config(secret)['services']['web']['environment']
        for key in DERIVED:
            with self.subTest(key=key):
                self.assertEqual((unset.get(key), given.get(key)), ('', 'on'))
        for key, value in given.items():
            with self.subTest(key=key):
                self.assertNotIn('example.test', value)

    def test_web_mounts_the_legal_texts_read_only(self):
        mounts = [volume for volume in web().get('volumes', [])
                  if volume['target'] == '/etc/gelabber/legal']
        self.assertEqual(len(mounts), 1)
        self.assertIs(mounts[0].get('read_only'), True)
        self.assertEqual(Path(mounts[0]['source']), ROOT / 'legal')
        self.assertTrue((ROOT / 'legal/README.md').is_file())


class Caddy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        caddy = _find_caddy()
        cls.adapted = {name: _adapt((ROOT / name).read_text(), caddy) for name in WEB_UPSTREAMS}

    def test_both_caddyfiles_send_the_public_paths_to_web(self):
        for name, (upstream, host) in WEB_UPSTREAMS.items():
            for path in PUBLIC_PATHS:
                with self.subTest(caddyfile=name, path=path):
                    self.assertEqual(route(self.adapted[name], path, host), upstream)

    def test_the_api_route_of_the_form_goes_to_the_api(self):
        for name, (upstream, host) in WEB_UPSTREAMS.items():
            with self.subTest(caddyfile=name):
                self.assertNotIn(route(self.adapted[name], '/api/public/feedback', host), (upstream, None))


class Files(unittest.TestCase):
    def test_the_start_script_reads_only_these_settings(self):
        read = set(re.findall(r'\$\{((?:GELABBER_|APP_)[A-Z0-9_]*)', SCRIPT.read_text()))
        self.assertEqual(sorted(read - SCRIPT_TEST_ONLY), sorted(SETTINGS | DERIVED.keys()))

    def test_the_examples_list_every_setting(self):
        for example in ('.env.example', '.env.homelab.example'):
            keys = set(re.findall(r'^#?([A-Z][A-Z0-9_]*)=', (ROOT / example).read_text(), re.M))
            with self.subTest(example=example):
                self.assertEqual(sorted(SETTINGS - keys), [])

    @unittest.skipUnless(shutil.which('git') and (REPO / '.git').exists(), 'no git checkout')
    def test_operator_texts_stay_out_of_commits(self):
        def ignored(path):
            return subprocess.run(['git', '-C', str(REPO), 'check-ignore', '-q', '--no-index',
                                   path]).returncode == 0
        self.assertTrue(ignored('deploy/compose/legal/privacy.de.html'))
        self.assertTrue(ignored('deploy/compose/legal/imprint.en.html'))
        self.assertFalse(ignored('deploy/compose/legal/README.md'))


if __name__ == '__main__':
    unittest.main()
