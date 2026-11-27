"""Flaky tests are retried, reported and recorded, never silent
(.github/workflows/scripts/flaky_tests.py, GEL-389).

The fixture is a JUnit report that cargo-nextest 0.9.149 wrote with
`--profile ci` for three tests: one passes, one fails once and then passes,
one fails both times.
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/flaky_tests.py'
FIXTURE = Path(__file__).resolve().parent / 'fixtures/nextest-retry-junit.xml'


def load():
    spec = importlib.util.spec_from_file_location('flaky_tests', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


flaky_tests = load()
RUN = {'url': 'https://github.com/o/r/actions/runs/1', 'branch': 'v0.14', 'sha': 'abc123'}


def entries(count):
    return ''.join(f'[[test]]\nid = "crate::binary test_{n}"\nissue = {100 + n}\n' for n in range(count))


class Reports(unittest.TestCase):
    def test_only_a_test_that_passed_on_retry_is_flaky(self):
        found = flaky_tests.flakes(FIXTURE.read_text())
        self.assertEqual([flake.test for flake in found], ['flakycrate::t flaky'])
        self.assertEqual(found[0].attempts, 2)
        self.assertIn('first attempt fails', found[0].message)

    def test_a_missing_report_is_named(self):
        missing = str(FIXTURE.parent / 'missing.xml')
        self.assertEqual(flaky_tests.read_reports([missing, str(FIXTURE)]),
                         (flaky_tests.flakes(FIXTURE.read_text()), [missing]))

    def test_a_folder_holds_the_downloaded_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            nested = Path(tmp, 'junit-desktop-linux/ci')
            nested.mkdir(parents=True)
            (nested / 'junit.xml').write_text(FIXTURE.read_text())
            found, missing = flaky_tests.read_reports([tmp])
            self.assertEqual(([flake.test for flake in found], missing), (['flakycrate::t flaky'], []))


class Quarantine(unittest.TestCase):
    def test_the_repository_list_is_valid(self):
        known = flaky_tests.quarantine(flaky_tests.QUARANTINE.read_text())
        self.assertLessEqual(len(known), flaky_tests.CAP)

    def test_the_list_holds_at_most_five(self):
        self.assertEqual(len(flaky_tests.quarantine(entries(5))), 5)
        with self.assertRaisesRegex(ValueError, 'at most 5'):
            flaky_tests.quarantine(entries(6))

    def test_every_entry_names_its_issue(self):
        for entry in ('[[test]]\nid = "a b"\n', '[[test]]\nid = "a b"\nissue = "GEL-1"\n',
                      '[[test]]\nid = "a b"\nissue = 0\n', '[[test]]\nissue = 3\n'):
            with self.subTest(entry=entry), self.assertRaises(ValueError):
                flaky_tests.quarantine(entry)

    def test_a_test_is_listed_once(self):
        with self.assertRaisesRegex(ValueError, 'twice'):
            flaky_tests.quarantine('[[test]]\nid = "a b"\nissue = 1\n' * 2)


class Report(unittest.TestCase):
    FLAKES = [flaky_tests.Flake('api::gateway known', 2, "panicked at tests/gateway.rs:9:5:\nassertion `left == right` failed"),
              flaky_tests.Flake('ui new_one', 2, 'took 100%0A::error::forged\n::error::forged too')]

    def test_the_summary_names_every_flaky_test_and_where_it_is_tracked(self):
        text = flaky_tests.summary(self.FLAKES, {'api::gateway known': 1058}, 'Rust tests (1/2)')
        self.assertIn('| `api::gateway known` | 2 | #1058 |', text)
        self.assertRegex(text, r'\| `ui new_one` \| 2 \| not on \.github/flaky-tests\.toml')
        self.assertEqual(flaky_tests.summary([], {}, 'job'), '')

    def test_each_flaky_test_is_one_warning_on_one_line(self):
        lines = flaky_tests.warnings(self.FLAKES)
        self.assertEqual(len(lines), 2)
        for line in lines:
            self.assertTrue(line.startswith('::warning title=Flaky test::'))
            self.assertNotIn('\n', line)
        # The runner decodes %0A into a line break; the text must stay text.
        self.assertIn('took 100%250A::error::forged', lines[1])
        self.assertIn('%0A::error::forged too', lines[1])
        # Where it panicked, then what failed: the assertion is on line two.
        self.assertIn('%0Aassertion `left == right` failed', lines[0])

    def test_a_message_cannot_end_its_code_block(self):
        message = 'left ``` and ```` inside'
        block = flaky_tests.fence(message)
        fence = block.splitlines()[0]
        self.assertEqual(block, f'{fence}\n{message}\n{fence}')
        self.assertGreater(len(fence), max(len(run) for run in re.findall('`+', message)))


class Issues(unittest.TestCase):
    def test_a_listed_test_goes_to_its_issue(self):
        flake = flaky_tests.Flake('api::outbox wake', 2, 'boom')
        [(action, target, body)] = flaky_tests.plan([flake], {'api::outbox wake': 1058},
                                                    {'Flaky test: api::outbox wake': 7}, RUN)
        self.assertEqual((action, target), ('comment', 1058))
        self.assertIn(RUN['url'], body)
        self.assertIn('abc123', body)
        self.assertIn('boom', body)

    def test_an_unlisted_test_gets_an_issue_once(self):
        flake = flaky_tests.Flake('ui screens::x', 2, 'timed out waiting')
        [(action, target, body)] = flaky_tests.plan([flake, flake], {}, {}, RUN)
        self.assertEqual((action, target), ('open', 'Flaky test: ui screens::x'))
        self.assertIn('.github/flaky-tests.toml', body)
        # The next push run finds the issue it opened by its title.
        [(action, target, _)] = flaky_tests.plan([flake], {}, {target: 42}, RUN)
        self.assertEqual((action, target), ('comment', 42))

    def test_a_run_opens_at_most_three_issues(self):
        found = [flaky_tests.Flake(f'ui test_{n}', 2, 'slow start') for n in range(5)]
        actions = flaky_tests.plan(found + [flaky_tests.Flake('api known', 2, '')], {'api known': 1058}, {}, RUN)
        self.assertEqual([action for action, _, _ in actions], ['open'] * 3 + ['skip'] * 2 + ['comment'])

    def test_without_the_open_issues_no_issue_is_opened(self):
        found = [flaky_tests.Flake('ui new', 2, ''), flaky_tests.Flake('api known', 2, '')]
        actions = flaky_tests.plan(found, {'api known': 1058}, None, RUN)
        self.assertEqual([(action, target) for action, target, _ in actions],
                         [('skip', 'Flaky test: ui new'), ('comment', 1058)])

    def test_a_new_issue_asks_for_its_place_in_linear(self):
        [(_, _, body)] = flaky_tests.plan([flaky_tests.Flake('ui new', 2, '')], {}, {}, RUN)
        self.assertIn('Add this issue to the Linear project Gelabber, label Bug.', body)

    def test_a_long_test_name_still_finds_its_issue(self):
        flake = flaky_tests.Flake('x ' + 'y' * 300, 2, '')
        [(_, title, _)] = flaky_tests.plan([flake], {}, {}, RUN)
        self.assertEqual(len(title), 256)
        self.assertEqual(flaky_tests.plan([flake], {}, {title: 9}, RUN)[0][:2], ('comment', 9))


class FakeGitHub:
    """Answers `gh api` as GitHub would, and records each request."""

    def __init__(self, issues, fail=()):
        self.issues, self.fail, self.calls = issues, fail, []

    def __call__(self, *args):
        self.calls.append(args)
        endpoint = next(arg for arg in args if arg.startswith('repos/'))
        if any(part in endpoint for part in self.fail):
            raise subprocess.CalledProcessError(1, 'gh', stderr='HTTP 403: rate limit')
        if '--method' not in args:
            return json.dumps([self.issues])
        if endpoint.endswith('/comments'):
            return '{}'
        return json.dumps({'number': 500 + len(self.calls)})

    def posts(self):
        return [(next(a for a in call if a.startswith('repos/')), [a for a in call if a.startswith(('title=', 'body='))])
                for call in self.calls if '--method' in call]


ENV = {'GITHUB_REPOSITORY': 'o/r', 'GITHUB_SERVER_URL': 'https://github.com', 'GITHUB_RUN_ID': '1',
       'GITHUB_REF_NAME': 'v0.14', 'GITHUB_SHA': 'abc123'}


class Main(unittest.TestCase):
    """The script as CI runs it, with a stand-in for GitHub."""

    def run_main(self, argv, github=None, quarantine=''):
        with tempfile.TemporaryDirectory() as tmp:
            listed = Path(tmp, 'flaky-tests.toml')
            listed.write_text(quarantine)
            summary = Path(tmp, 'summary.md')
            out = io.StringIO()
            with mock.patch.dict(os.environ, {**ENV, 'GITHUB_STEP_SUMMARY': str(summary)}), \
                    mock.patch.object(flaky_tests, 'QUARANTINE', listed), \
                    mock.patch.object(flaky_tests, 'gh_api', github or FakeGitHub([])), \
                    contextlib.redirect_stdout(out):
                code = flaky_tests.main(argv)
            return code, out.getvalue(), summary.read_text() if summary.exists() else ''

    def test_summary_writes_the_table_with_the_listed_issue(self):
        code, out, summary = self.run_main(['summary', str(FIXTURE)],
                                           quarantine='[[test]]\nid = "flakycrate::t flaky"\nissue = 77\n')
        self.assertEqual(code, 0)
        self.assertIn('::warning title=Flaky test::flakycrate::t flaky failed', out)
        self.assertIn('| `flakycrate::t flaky` | 2 | #77 |', summary)

    def test_summary_fails_without_its_report(self):
        missing = str(FIXTURE.parent / 'missing.xml')
        code, out, _ = self.run_main(['summary', missing])
        self.assertEqual(code, 1)
        self.assertIn(f'::error title=Flaky tests::no JUnit report at {missing}', out)

    def test_issues_adds_a_flake_to_the_open_issue_of_its_test(self):
        github = FakeGitHub([{'number': 41, 'title': 'Flaky test: flakycrate::t flaky'},
                             {'number': 42, 'title': 'Flaky test: flakycrate::t flaky', 'pull_request': {}},
                             {'number': 43, 'title': 'Something else'}])
        code, _, summary = self.run_main(['issues', str(FIXTURE.parent / 'missing.xml'), str(FIXTURE)], github)
        self.assertEqual(code, 0)
        self.assertNotIn('--method', github.calls[0])
        self.assertIn('repos/o/r/issues?state=open&per_page=100', github.calls[0])
        [(endpoint, fields)] = github.posts()
        self.assertEqual(endpoint, 'repos/o/r/issues/41/comments')
        self.assertIn('https://github.com/o/r/actions/runs/1', fields[0])
        self.assertIn('- Added to #41', summary)

    def test_issues_comments_on_the_listed_issue_or_opens_one(self):
        github = FakeGitHub([])
        self.run_main(['issues', str(FIXTURE)], github, quarantine='[[test]]\nid = "flakycrate::t flaky"\nissue = 77\n')
        self.assertEqual([endpoint for endpoint, _ in github.posts()], ['repos/o/r/issues/77/comments'])
        github = FakeGitHub([])
        _, _, summary = self.run_main(['issues', str(FIXTURE)], github)
        [(endpoint, fields)] = github.posts()
        self.assertEqual(endpoint, 'repos/o/r/issues')
        self.assertIn('title=Flaky test: flakycrate::t flaky', fields)
        self.assertIn('- Opened #', summary)

    def test_a_failed_request_warns_and_the_run_goes_on(self):
        github = FakeGitHub([], fail=('issues/77/comments',))
        code, out, summary = self.run_main(['issues', str(FIXTURE)], github,
                                           quarantine='[[test]]\nid = "flakycrate::t flaky"\nissue = 77\n')
        self.assertEqual(code, 0)
        self.assertIn('::warning title=Flaky test not recorded::', out)
        self.assertIn('- Not recorded (GitHub request failed)', summary)

    def test_unknown_open_issues_open_none(self):
        github = FakeGitHub([], fail=('state=open',))
        code, out, _ = self.run_main(['issues', str(FIXTURE)], github)
        self.assertEqual(code, 0)
        self.assertIn('::warning title=Open issues unknown::', out)
        self.assertEqual(github.posts(), [])


class Workflows(unittest.TestCase):
    """CI runs the retrying profile and reads the report where it is written."""

    def test_ci_retries_and_reads_each_workspace_report(self):
        # nextest writes its reports under <workspace>/target/nextest, also
        # when CARGO_TARGET_DIR points elsewhere (run-native-ci.sh).
        cases = (('backend', '.github/workflows/ci.yml'), ('desktop', '.github/workflows/desktop-native.yml'))
        for workspace, workflow in cases:
            profiles = tomllib.loads((ROOT / workspace / '.config/nextest.toml').read_text())['profile']
            text = (ROOT / workflow).read_text()
            summary = re.search(r'flaky_tests\.py summary (.*)\n', text).group(1).split()
            upload = text.split('- name: Upload the test report\n', 1)[1].split('\n      - name:', 1)[0]
            used = sorted(set(re.findall(r'nextest run --profile (\S+) ', text)))
            self.assertIn('ci', used)
            for name in used:
                with self.subTest(workspace=workspace, profile=name):
                    profile = dict(profiles['ci'], **profiles[name])
                    self.assertGreaterEqual(profile['retries'], 1)
                    self.assertNotEqual(profile.get('flaky-result'), 'fail')
                    report = f'{workspace}/target/nextest/{name}/{profile["junit"]["path"]}'
                    self.assertIn(report, summary)
                    self.assertIn(report, upload)

    def test_push_runs_record_the_reports_of_both_workspaces(self):
        ci = (ROOT / '.github/workflows/ci.yml').read_text()
        job = ci.split('\n  flaky-tests:\n', 1)[1].split('\n  ci-ok:', 1)[0]
        self.assertIn('needs: [rust, desktop]', job)
        self.assertIn('pattern: junit-*', job)
        self.assertIn('flaky_tests.py issues junit\n', job)
        self.assertIn("github.event_name == 'push'", job)
        for name in ('junit-rust-${{ matrix.part }}', 'junit-desktop-linux'):
            self.assertIn(f'name: {name}\n', ci + (ROOT / '.github/workflows/desktop-native.yml').read_text())


if __name__ == '__main__':
    unittest.main()
