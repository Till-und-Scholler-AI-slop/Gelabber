"""Flaky tests are retried, reported and recorded, never silent
(.github/workflows/scripts/flaky_tests.py, GEL-389).

The fixture is a JUnit report that cargo-nextest 0.9.149 wrote with
`--profile ci` for three tests: one passes, one fails once and then passes,
one fails both times.
"""
import importlib.util
from pathlib import Path
import re
import tomllib
import unittest

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

    def test_a_job_without_a_report_has_no_flakes(self):
        # A job that failed before its tests ran wrote no report.
        self.assertEqual(flaky_tests.read_reports([str(FIXTURE.parent / 'missing.xml'), str(FIXTURE)]),
                         flaky_tests.flakes(FIXTURE.read_text()))


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
    FLAKES = [flaky_tests.Flake('api::gateway known', 2, 'timed out'),
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
        self.assertNotIn('forged too', lines[1])

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

    def test_a_long_test_name_still_finds_its_issue(self):
        flake = flaky_tests.Flake('x ' + 'y' * 300, 2, '')
        [(_, title, _)] = flaky_tests.plan([flake], {}, {}, RUN)
        self.assertEqual(len(title), 256)
        self.assertEqual(flaky_tests.plan([flake], {}, {title: 9}, RUN)[0][:2], ('comment', 9))


class Workflows(unittest.TestCase):
    """CI runs the retrying profile and reads the report where it is written."""

    def test_ci_retries_and_reads_each_workspace_report(self):
        # nextest writes its reports under <workspace>/target/nextest, also
        # when CARGO_TARGET_DIR points elsewhere (run-native-ci.sh).
        cases = (('backend', '.github/workflows/ci.yml'), ('desktop', '.github/workflows/desktop-native.yml'))
        for workspace, workflow in cases:
            with self.subTest(workspace=workspace):
                profile = tomllib.loads((ROOT / workspace / '.config/nextest.toml').read_text())['profile']['ci']
                self.assertGreaterEqual(profile['retries'], 1)
                self.assertNotEqual(profile.get('flaky-result'), 'fail')
                text = (ROOT / workflow).read_text()
                self.assertRegex(text, r'nextest run --profile ci ')
                report = f'{workspace}/target/nextest/ci/{profile["junit"]["path"]}'
                self.assertIn(f'flaky_tests.py summary {report}\n', text)
                self.assertIn(f'path: {report}\n', text)

    def test_push_runs_record_the_reports_of_both_workspaces(self):
        ci = (ROOT / '.github/workflows/ci.yml').read_text()
        job = ci.split('\n  flaky-tests:\n', 1)[1].split('\n  ci-ok:', 1)[0]
        self.assertIn('needs: [rust, desktop]', job)
        self.assertIn('pattern: junit-*', job)
        self.assertIn('flaky_tests.py issues junit/*/*.xml', job)
        for name in ('junit-rust-${{ matrix.part }}', 'junit-desktop-linux'):
            self.assertIn(f'name: {name}\n', ci + (ROOT / '.github/workflows/desktop-native.yml').read_text())


if __name__ == '__main__':
    unittest.main()
