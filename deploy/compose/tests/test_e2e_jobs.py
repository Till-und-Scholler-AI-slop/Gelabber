"""Which jobs of e2e-extended.yml a push starts, and the outcome its issue
reports (.github/workflows/scripts/e2e_jobs.py, GEL-376)."""
import importlib.util
import re
import subprocess
import sys
import unittest

import workflow_data

SCRIPT = workflow_data.ROOT / '.github/workflows/scripts/e2e_jobs.py'


def load():
    spec = importlib.util.spec_from_file_location('e2e_jobs', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


e2e_jobs = load()


class Plan(unittest.TestCase):
    def test_each_change_starts_the_jobs_it_concerns(self):
        both, faults, desktop = {'faults', 'desktop'}, {'faults'}, {'desktop'}
        for path, expected in (
                ('backend/api/src/main.rs', both),
                ('web/src/main.tsx', faults),
                ('web/scripts/parity/seed.mjs', both),
                ('web/package-lock.json', both),
                ('desktop/client/src/lib.rs', desktop),
                ('desktop/client/tests/fixtures/me.json', desktop),
                ('.github/workflows/e2e-extended.yml', both),
                ('.github/workflows/scripts/e2e_jobs.py', both),
                ('.github/workflows/scripts/apt-install.sh', both),
                ('deploy/compose/compose.yaml', both),
                ('docs/README.md', set()),
                ('deploy/backup.sh', set()),
                ('.github/workflows/ci.yml', set())):
            with self.subTest(path=path):
                self.assertEqual(e2e_jobs.plan([path]), expected)

    def test_a_desktop_only_push_leaves_the_browsers_alone(self):
        self.assertEqual(e2e_jobs.plan(['desktop/ui/src/app.rs', 'desktop/README.md']), {'desktop'})

    def test_all_starts_both(self):
        out = subprocess.run([sys.executable, str(SCRIPT), '--all'], input='',
                             capture_output=True, text=True, check=True).stdout
        self.assertEqual(out.split(), ['faults=true', 'desktop=true'])

    def test_paths_on_stdin_give_the_outputs(self):
        out = subprocess.run([sys.executable, str(SCRIPT)], input='web/src/main.tsx\n',
                             capture_output=True, text=True, check=True).stdout
        self.assertEqual(out.split(), ['faults=true', 'desktop=false'])

    def test_every_path_that_starts_a_job_starts_the_workflow(self):
        paths = workflow_data.load('e2e-extended.yml')['on']['push']['paths']
        listed = {path.removesuffix('**') for path in paths}
        for prefix in {p for prefixes in e2e_jobs.JOBS.values() for p in prefixes}:
            with self.subTest(prefix=prefix):
                self.assertTrue(any(prefix.startswith(p) for p in listed), prefix)


class Outcome(unittest.TestCase):
    def test_the_decision_table(self):
        for plan, faults, desktop, expected in (
                ('success', 'success', 'success', 'success'),
                ('success', 'skipped', 'success', 'success'),
                ('success', 'success', 'skipped', 'success'),
                ('success', 'failure', 'success', 'failure'),
                ('success', 'skipped', 'failure', 'failure'),
                # A job past its timeout-minutes is cancelled: it fails the run.
                ('success', 'success', 'cancelled', 'failure'),
                ('success', 'cancelled', 'skipped', 'failure'),
                ('failure', 'skipped', 'skipped', 'failure'),
                ('success', 'skipped', 'skipped', 'other')):
            with self.subTest(results=(plan, faults, desktop)):
                self.assertEqual(e2e_jobs.result(plan, faults, desktop), expected)

    def test_the_issue_job_reads_the_outcome(self):
        jobs = workflow_data.load('e2e-extended.yml')['jobs']
        self.assertEqual(set(jobs['outcome']['needs']), {'plan', 'faults', 'desktop-live'})
        self.assertEqual(jobs['report']['needs'], 'outcome')
        step = jobs['report']['steps'][0]
        self.assertEqual(step['env']['RESULT'], '${{ needs.outcome.outputs.result }}')


class Libwebrtc(unittest.TestCase):
    def test_desktop_live_looks_for_the_package_desktop_native_builds(self):
        # desktop-native.yml stores the package under this key; desktop-live
        # fetches it, so a key input changed in one place only finds nothing.
        def key(workflow):
            text = (workflow_data.WORKFLOWS / workflow).read_text()
            return re.findall(r'key=libwebrtc-linux-x64-\$\{\{ hashFiles\([^)]*\) \}\}', text)

        native, live = key('desktop-native.yml'), key('e2e-extended.yml')
        self.assertEqual(len(live), 1)
        self.assertIn(live[0], native)


if __name__ == '__main__':
    unittest.main()
