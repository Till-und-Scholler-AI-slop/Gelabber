"""Which jobs of e2e-extended.yml a push starts, and the outcome its issue
reports (.github/workflows/scripts/e2e_jobs.py, GEL-376)."""
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
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
                ('web/src/main.tsx', both),
                ('rust-toolchain.toml', both),
                ('web/scripts/parity/seed.mjs', both),
                ('web/package-lock.json', both),
                ('desktop/client/src/lib.rs', desktop),
                ('desktop/client/tests/fixtures/me.json', desktop),
                ('.github/workflows/e2e-extended.yml', both),
                ('.github/workflows/scripts/e2e_jobs.py', both),
                ('.github/workflows/scripts/apt-install.sh', both),
                ('.github/workflows/scripts/start-minio.sh', both),
                ('.github/workflows/scripts/playwright-browsers.sh', both),
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
        out = subprocess.run([sys.executable, str(SCRIPT)], input='desktop/README.md\n',
                             capture_output=True, text=True, check=True).stdout
        self.assertEqual(out.split(), ['faults=false', 'desktop=true'])

    def test_every_path_that_starts_a_job_starts_the_workflow(self):
        paths = workflow_data.load('e2e-extended.yml')['on']['push']['paths']
        listed = {path.removesuffix('**') for path in paths}
        for prefix in {p for prefixes in e2e_jobs.JOBS.values() for p in prefixes}:
            with self.subTest(prefix=prefix):
                self.assertTrue(any(prefix.startswith(p) for p in listed), prefix)


class PushPlan(unittest.TestCase):
    """`plan <event> <before>` in a shallow clone of the pushed branch, as the
    runner's checkout has it, against an origin that holds `before`."""

    def setUp(self):
        tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@example.test',
                        GIT_COMMITTER_NAME='t', GIT_COMMITTER_EMAIL='t@example.test',
                        GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1')
        work, origin, self.clone = tmp / 'work', tmp / 'origin.git', tmp / 'clone'
        self.git('init', '--quiet', '--initial-branch=v0.14', str(work))
        self.commits = []
        for path in ('web/src/main.tsx', 'desktop/client/src/lib.rs'):
            (work / path).parent.mkdir(parents=True, exist_ok=True)
            (work / path).write_text(path)
            self.git('-C', str(work), 'add', path)
            self.git('-C', str(work), 'commit', '--quiet', '-m', path)
            self.commits.append(self.git('-C', str(work), 'rev-parse', 'HEAD').strip())
        self.git('clone', '--quiet', '--bare', str(work), str(origin))
        self.git('clone', '--quiet', '--depth=1', f'file://{origin}', str(self.clone))

    def git(self, *args):
        return subprocess.run(['git', *args], env=self.env, check=True,
                              capture_output=True, text=True).stdout

    def plan(self, event, before):
        out = subprocess.run([sys.executable, str(SCRIPT), 'plan', event, before], cwd=self.clone,
                             env=self.env, capture_output=True, text=True, check=True).stdout
        return out.split()

    def test_a_push_starts_the_jobs_its_commits_concern(self):
        self.assertEqual(self.plan('push', self.commits[0]), ['faults=false', 'desktop=true'])

    def test_without_a_known_starting_point_both_run(self):
        for event, before in (('push', '0' * 40), ('push', 'f' * 40), ('push', ''),
                              ('workflow_dispatch', self.commits[0]),
                              ('pull_request', self.commits[0])):
            with self.subTest(event=event, before=before):
                self.assertEqual(self.plan(event, before), ['faults=true', 'desktop=true'])


class Outcome(unittest.TestCase):
    def test_the_decision_table(self):
        for plan, faults, desktop, expected in (
                ('success', 'success', 'success', 'success'),
                # A skipped job says nothing about itself: the issue stays.
                ('success', 'skipped', 'success', 'other'),
                ('success', 'success', 'skipped', 'other'),
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
        # It only reads, so every run shows it; only the issue is for pushes.
        self.assertEqual(jobs['outcome']['if'], '${{ !cancelled() }}')
        self.assertEqual(jobs['report']['needs'], 'outcome')
        step = jobs['report']['steps'][0]
        self.assertEqual(step['env']['RESULT'], '${{ needs.outcome.outputs.result }}')


class PreviousMinor(unittest.TestCase):
    def test_the_desktop_tests_also_run_against_the_previous_minor(self):
        # The app supports servers of its own and the previous minor (GEL-436).
        job = workflow_data.load('e2e-extended.yml')['jobs']['desktop-live']
        self.assertEqual(job['strategy']['matrix']['server'], ['this tree', 'previous minor'])
        self.assertIs(job['strategy']['fail-fast'], False)

    def test_the_previous_release_follows_the_one_rule(self):
        # previous_release.sh decides it for the upgrade test too; no second rule.
        steps = workflow_data.load('e2e-extended.yml')['jobs']['desktop-live']['steps']
        step = next(s for s in steps if s.get('id') == 'previous')
        self.assertIn('.github/workflows/scripts/previous_release.sh --fetch', step['run'])
        self.assertNotIn('git tag', step['run'])


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
