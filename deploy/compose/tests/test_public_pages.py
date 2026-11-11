"""The public pages (#522) reach the web container: compose passes every value
the web image's start script reads and mounts the operator's texts, those
texts stay out of commits, and both Caddyfiles hand the pages' paths to web."""
from pathlib import Path
import re
import shutil
import subprocess
import unittest

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
PUBLIC_PATHS = ('/', '/de/', '/en/', '/de/impressum', '/en/privacy', '/robots.txt',
                '/sitemap.xml', '/public-pages.json')
WEB_UPSTREAMS = {'Caddyfile': 'web:80',
                 'Caddyfile.homelab': '{$GELABBER_WEB_UPSTREAM:127.0.0.1:8088}'}


def web_service():
    return re.search(r'^  web:\n(.*?)^  \S', (ROOT / 'compose.yaml').read_text(),
                     re.M | re.S).group(1)


def web_environment():
    env = re.search(r'^    environment:\n((?:      .*\n|\s*\n)*)', web_service(), re.M).group(1)
    return dict(re.findall(r'^      ([A-Z][A-Z0-9_]*): (.*)$', env, re.M))


def site_handles(text):
    """(paths, upstream) of each `handle` in the order Caddy tries them; a
    handle without a matcher has paths None."""
    matchers = {name: paths.split() for name, paths in
                re.findall(r'^\s*(@\w+) path (.+)$', text, re.M)}
    handles = []
    for match in re.finditer(r'^\thandle(?: (\S+))? \{\n(.*?)^\t\}', text, re.M | re.S):
        target, body = match.groups()
        upstream = re.search(r'reverse_proxy (\S+)', body)
        paths = None if target is None else matchers.get(target, [target])
        handles.append((paths, upstream and upstream.group(1)))
    return handles


def caddy_path_matches(pattern, path):
    if pattern.endswith('*'):
        return path.startswith(pattern[:-1])
    return path == pattern


class PublicPages(unittest.TestCase):
    def test_web_receives_every_setting_with_an_empty_default(self):
        env = web_environment()
        for key in sorted(SETTINGS):
            with self.subTest(key=key):
                self.assertEqual(env.get(key), '${' + key + ':-}')

    def test_the_start_script_reads_only_these_settings(self):
        read = set(re.findall(r'\$\{((?:GELABBER_|APP_)[A-Z0-9_]*)', SCRIPT.read_text()))
        self.assertEqual(sorted(read - SCRIPT_TEST_ONLY), sorted(SETTINGS))

    def test_web_mounts_the_legal_texts_read_only(self):
        self.assertIn('      - ./legal:/etc/gelabber/legal:ro\n', web_service())
        self.assertTrue((ROOT / 'legal/README.md').is_file())

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

    def test_both_caddyfiles_send_the_public_paths_to_web(self):
        for name, upstream in WEB_UPSTREAMS.items():
            handles = site_handles((ROOT / name).read_text())
            self.assertGreaterEqual(len(handles), 7, name)
            self.assertEqual(handles[-1], (None, upstream))
            for path in PUBLIC_PATHS:
                with self.subTest(caddyfile=name, path=path):
                    first = next(target for paths, target in handles
                                 if paths is None
                                 or any(caddy_path_matches(p, path) for p in paths))
                    self.assertEqual(first, upstream)


if __name__ == '__main__':
    unittest.main()
