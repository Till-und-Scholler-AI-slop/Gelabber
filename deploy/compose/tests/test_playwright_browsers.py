"""playwright-browsers.sh installs the browsers' system packages through
apt-install.sh, then the browsers (.github/workflows/scripts, GEL-470).

The real script runs from a copy of .github/workflows/scripts with a
stand-in apt-install.sh beside it and a stand-in npx on PATH. Like
Playwright's, the stand-in's `install-deps --dry-run` lists the missing
packages and exits 1, or says all are installed and exits 0; after the
install, it lists what is still missing (STILL_MISSING).
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/playwright-browsers.sh'

# npx playwright install-deps --dry-run <browser>... | npx playwright install <browser>...
FAKE_NPX = r'''#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as calls:
    calls.write('npx ' + ' '.join(args) + '\n')
if args[:3] == ['playwright', 'install-deps', '--dry-run']:
    installed = any(line.startswith('apt-install.sh') for line in open(os.environ['CALLS']))
    missing = os.environ['STILL_MISSING' if installed else 'MISSING'].split()
    if os.environ.get('BROKEN'):
        print('Error: apt-get install -s exited with code 100', file=sys.stderr)
        sys.exit(1)
    if not missing:
        print('All system dependencies are installed.')
        sys.exit(0)
    print(f'Missing system dependencies ({len(missing)}):')
    print('\n'.join(f'  {p}' for p in sorted(missing)))
    sys.exit(1)
if args[:2] == ['playwright', 'install']:
    sys.exit(0)
sys.exit(f'stand-in npx: unexpected call {args}')
'''
FAKE_APT_INSTALL = '#!/bin/sh\necho "apt-install.sh $*" >> "$CALLS"\n'


class PlaywrightBrowsers(unittest.TestCase):
    def run_script(self, *browsers, missing=(), still_missing=(), broken=False):
        tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        scripts = tmp / '.github/workflows/scripts'
        scripts.mkdir(parents=True)
        (tmp / 'web').mkdir()
        shutil.copy(SCRIPT, scripts)
        (scripts / 'apt-install.sh').write_text(FAKE_APT_INSTALL)
        (scripts / 'apt-install.sh').chmod(0o755)
        bin_dir = tmp / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'npx').write_text(FAKE_NPX)
        (bin_dir / 'npx').chmod(0o755)
        calls = tmp / 'calls'
        calls.touch()
        env = dict(os.environ, PATH=f'{bin_dir}:{os.environ["PATH"]}', CALLS=str(calls),
                   MISSING=' '.join(missing), STILL_MISSING=' '.join(still_missing),
                   BROKEN='1' if broken else '')
        result = subprocess.run(['bash', str(scripts / 'playwright-browsers.sh'), *browsers],
                                env=env, capture_output=True, text=True, timeout=60)
        return result, calls.read_text().splitlines()

    def test_missing_packages_go_through_apt_install(self):
        result, calls = self.run_script('chromium', 'firefox', missing=['libnss3', 'fonts-unifont'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, ['npx playwright install-deps --dry-run chromium firefox',
                                 'apt-install.sh fonts-unifont libnss3',
                                 'npx playwright install-deps --dry-run chromium firefox',
                                 'npx playwright install chromium firefox'])

    def test_a_package_still_missing_after_the_install_fails(self):
        result, calls = self.run_script('chromium', missing=['libnss3', 'fonts-unifont'],
                                        still_missing=['libnss3'])
        self.assertEqual(result.returncode, 1)
        self.assertIn('  libnss3', result.stdout)
        self.assertNotIn('npx playwright install chromium', calls)

    def test_with_every_package_there_only_the_browsers_install(self):
        result, calls = self.run_script('chromium')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, ['npx playwright install-deps --dry-run chromium',
                                 'npx playwright install-deps --dry-run chromium',
                                 'npx playwright install chromium'])

    def test_a_failing_dry_run_stops_before_any_install(self):
        result, calls = self.run_script('chromium', broken=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('apt-get install -s exited with code 100', result.stderr)
        self.assertEqual(calls, ['npx playwright install-deps --dry-run chromium'])


if __name__ == '__main__':
    unittest.main()
