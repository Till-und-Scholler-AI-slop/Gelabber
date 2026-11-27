"""The deployment tests run with the PyYAML that requirements.txt pins, CI
installs exactly that, and Dependabot keeps it current (GEL-403). Without
PyYAML the suite fails on import; it never skips."""
from pathlib import Path
import re
import unittest

import yaml

import workflow_data

ROOT = Path(__file__).resolve().parents[3]
REQUIREMENTS = 'deploy/compose/tests/requirements.txt'


def pinned():
    return re.search(r'^PyYAML==(\S+) ', (ROOT / REQUIREMENTS).read_text(), re.M).group(1)


class PinnedPyYAML(unittest.TestCase):
    def test_the_tests_run_with_the_pinned_version(self):
        self.assertEqual(yaml.__version__, pinned(),
                         'install it, and run the tests with PYTHONPATH=~/.cache/gelabber-pyyaml: '
                         f'python3 -m pip install --require-hashes --target ~/.cache/gelabber-pyyaml -r {REQUIREMENTS}')

    def test_ci_installs_the_pinned_files_before_the_tests_run(self):
        steps = workflow_data.load('ci.yml')['jobs']['deploy']['steps']
        names = [step.get('name') for step in steps]
        install = steps[names.index("Install the deployment tests' PyYAML")]
        tests = steps[names.index('Test deployment config')]
        self.assertLess(names.index("Install the deployment tests' PyYAML"), names.index('Test deployment config'))
        self.assertIn('--require-hashes', install['run'].split())
        self.assertIn(f'-r {REQUIREMENTS}', install['run'])
        target = re.search(r'--target "\$RUNNER_TEMP/(\S+)"', install['run']).group(1)
        self.assertEqual(tests['env']['PYTHONPATH'], f'${{{{ runner.temp }}}}/{target}')

    def test_dependabot_updates_the_pin(self):
        updates = yaml.safe_load((ROOT / '.github/dependabot.yml').read_text())['updates']
        folders = [entry.get('directory') for entry in updates if entry['package-ecosystem'] == 'pip']
        self.assertIn('/' + str(Path(REQUIREMENTS).parent), folders)


if __name__ == '__main__':
    unittest.main()
