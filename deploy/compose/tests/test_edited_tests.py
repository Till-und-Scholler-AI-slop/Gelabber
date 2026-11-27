"""The report of test edits to look at (.github/workflows/scripts/edited_tests.py),
run as ci.yml runs it, on scratch repositories."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/edited_tests.py'
ENV = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}


def run(before, after):
    """The script's annotations as (kind, path, line, text), and the job
    summary, for a change from the files `before` to `after`."""
    with tempfile.TemporaryDirectory() as tmp:
        def git(*args):
            subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args],
                           cwd=tmp, env=ENV, check=True, capture_output=True)

        def write(files):
            for path, text in files.items():
                target = Path(tmp) / path
                if text is None:
                    target.unlink()
                elif isinstance(text, bytes):
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(text)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(textwrap.dedent(text))
            git('add', '-A')
            git('commit', '-q', '--allow-empty', '-m', 'x')

        git('init', '-q')
        write(before)
        write(after)
        summary = Path(tmp) / 'summary.md'
        result = subprocess.run([sys.executable, str(SCRIPT), 'HEAD^1', 'HEAD'], cwd=tmp,
                                env={**ENV, 'GITHUB_STEP_SUMMARY': str(summary)},
                                capture_output=True, text=True, check=True)
        found = re.findall(r'^::warning file=([^,]+),line=(\d+),title=Test change%3A ([^:]+)::(.*)$',
                           result.stdout, re.M)
        run.stdout = result.stdout
        return [(unescape(kind), unescape(path), int(line), unescape(text))
                for path, line, kind, text in found], summary.read_text()


def unescape(value):
    """A workflow command's value as GitHub reads it."""
    for code, char in (('%0D', '\r'), ('%0A', '\n'), ('%3A', ':'), ('%2C', ','), ('%25', '%')):
        value = value.replace(code, char)
    return value


def kinds(before, after):
    return sorted((kind, path) for kind, path, _, _ in run(before, after)[0])


RUST_TEST = '''\
    #[tokio::test]
    async fn ledger_keeps_the_cap() {
        let ledger = usage().await;
        assert_eq!(ledger, (4, 0));
        assert_eq!(count(), 3, "three messages");
    }
'''


class Weakening(unittest.TestCase):
    def test_an_exact_check_that_becomes_one_of_two(self):
        # c96b8e0e: the ledger check became "either of two states".
        after = RUST_TEST.replace('assert_eq!(ledger, (4, 0));',
                                  'assert!(matches!(ledger, (4, 0) | (0, 4)), "quota lost");')
        found, _ = run({'api/tests/storage.rs': RUST_TEST}, {'api/tests/storage.rs': after})
        self.assertEqual([(kind, line) for kind, _, line, _ in found], [('assertion loosened', 4)])

    def test_to_equal_that_becomes_to_contain(self):
        before = 'it("lists them", () => {\n  expect(ids()).toEqual(["a", "c"]);\n});\n'
        after = 'it("lists them", () => {\n  expect(ids()).toContain("a");\n});\n'
        self.assertEqual(kinds({'web/src/a.test.ts': before}, {'web/src/a.test.ts': after}),
                         [('assertion loosened', 'web/src/a.test.ts')])

    def test_an_exact_value_that_becomes_a_list_it_must_be_in(self):
        before = 'it("counts", () => {\n  expect(count()).toBe(1);\n});\n'
        after = 'it("counts", () => {\n  expect([1, 2].includes(count())).toBe(true);\n});\n'
        self.assertEqual(kinds({'web/src/a.test.ts': before}, {'web/src/a.test.ts': after}),
                         [('assertion loosened', 'web/src/a.test.ts')])

    def test_an_assertion_deleted_from_a_test_that_stays(self):
        # c9c4af31: "sends all its layers" went, the test stayed.
        after = RUST_TEST.replace('        assert_eq!(count(), 3, "three messages");\n', '')
        found, _ = run({'core/tests/loopback.rs': RUST_TEST}, {'core/tests/loopback.rs': after})
        self.assertEqual([(kind, text) for kind, _, _, text in found],
                         [('assertion removed', 'assert_eq!(count(), 3, "three messages");')])

    def test_a_deleted_test(self):
        found, summary = run({'web/src/a.test.ts': 'it("one", () => {});\nit("two", () => {});\n'},
                             {'web/src/a.test.ts': 'it("one", () => {});\n'})
        self.assertEqual([(kind, text) for kind, _, _, text in found], [('test deleted', 'two')])
        self.assertIn('| <code>web/src/a.test.ts:2</code> | test deleted | <code>two</code> |', summary)

    def test_a_raised_limit(self):
        # 0ab2d6ad: 50 ms became 500 ms.
        before = 'it("is fast", () => {\n  expect(took()).toBeLessThan(50);\n});\n'
        found, _ = run({'web/src/md.test.ts': before},
                       {'web/src/md.test.ts': before.replace('50', '500')})
        self.assertEqual([kind for kind, *_ in found], ['limit raised'])

    def test_a_timeout_sleep_and_retry_loop_in_an_existing_test(self):
        # 5cae5bc3 gave a wait 5 s; 80cfcfcc clicked until the dialog opened.
        before = 'it("rebuilds", async () => {\n  await vi.waitFor(() => expect(x()).toBeTruthy());\n});\n'
        after = ('it("rebuilds", async () => {\n'
                 '  await vi.waitFor(() => expect(x()).toBeTruthy(), {\n    timeout: 5000,\n  });\n});\n')
        self.assertEqual(kinds({'web/src/s.test.ts': before}, {'web/src/s.test.ts': after}),
                         [('timeout added', 'web/src/s.test.ts')])
        rust = '#[cfg(test)]\nmod tests {\n    #[test]\n    fn opens() {\n        click();\n    }\n}\n'
        looped = rust.replace('        click();\n', '        while dialog().is_none() {\n'
                              '            click();\n            std::thread::sleep(TICK);\n        }\n')
        self.assertEqual(kinds({'ui/src/settings.rs': rust}, {'ui/src/settings.rs': looped}),
                         [('retry loop added', 'ui/src/settings.rs'), ('sleep added', 'ui/src/settings.rs')])

    def test_a_new_skip(self):
        self.assertEqual(kinds({'web/src/a.test.ts': 'it("one", () => {});\n'},
                               {'web/src/a.test.ts': 'it.skip("one", () => {});\n'}),
                         [('skip added', 'web/src/a.test.ts')])
        after = RUST_TEST.replace('    #[tokio::test]\n', '    #[tokio::test]\n    #[ignore]\n')
        self.assertEqual(kinds({'api/tests/x.rs': RUST_TEST}, {'api/tests/x.rs': after}),
                         [('skip added', 'api/tests/x.rs')])

    def test_an_early_return(self):
        after = RUST_TEST.replace('        let ledger', '        if slow() {\n            return;\n        }\n'
                                  '        let ledger')
        self.assertEqual(kinds({'api/tests/x.rs': RUST_TEST}, {'api/tests/x.rs': after}),
                         [('early return', 'api/tests/x.rs')])

    def test_the_runner_configuration(self):
        config = 'export default { test: { testTimeout: 5000 } };\n'
        self.assertEqual(kinds({'web/vitest.config.ts': config},
                               {'web/vitest.config.ts': config.replace('5000', '20000')}),
                         [('runner configuration', 'web/vitest.config.ts')])
        package = '{\n  "scripts": {\n    "test": "vitest run"\n  }\n}\n'
        self.assertEqual(kinds({'web/package.json': package},
                               {'web/package.json': package.replace('vitest run', 'vitest run --bail 0')}),
                         [('runner configuration', 'web/package.json')])


class Quiet(unittest.TestCase):
    def test_new_tests_with_their_waits(self):
        added = RUST_TEST.replace('ledger_keeps_the_cap', 'second_upload_waits') \
            .replace('        let ledger', '        tokio::time::sleep(TICK).await;\n        let ledger')
        self.assertEqual(run({'api/tests/x.rs': RUST_TEST}, {'api/tests/x.rs': RUST_TEST + '\n' + added})[0], [])
        self.assertEqual(run({}, {'web/src/new.test.ts': 'it("a", async () => {\n  await sleep(10);\n'
                                                          '  expect(a()).toEqual(1);\n});\n'})[0], [])

    def test_a_stricter_check(self):
        after = RUST_TEST.replace('assert_eq!(count(), 3, "three messages");',
                                  'assert_eq!(count(), 3, "three messages, in order");')
        self.assertEqual(run({'api/tests/x.rs': RUST_TEST}, {'api/tests/x.rs': after})[0], [])

    def test_a_test_moved_to_another_file(self):
        self.assertEqual(run({'api/tests/a.rs': RUST_TEST, 'api/tests/b.rs': ''},
                             {'api/tests/a.rs': None, 'api/tests/b.rs': RUST_TEST})[0], [])

    def test_production_code_and_a_new_test_script(self):
        code = '#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {\n        assert_eq!(f(), 1);\n    }\n}\n'
        changed = 'fn f() -> u8 {\n    1\n}\n\n' + code.replace('assert_eq!(f(), 1)', 'assert_eq!(f(), 1)')
        source = 'fn f() -> u8 {\n    loop {\n        return 1;\n    }\n}\n\n' + code
        self.assertEqual(run({'ui/src/lib.rs': changed}, {'ui/src/lib.rs': source})[0], [])
        package = '{\n  "scripts": {\n    "test": "vitest run"\n  }\n}\n'
        self.assertEqual(run({'web/package.json': package}, {'web/package.json': package.replace(
            '"test": "vitest run"', '"test": "vitest run",\n    "test:smoke": "node smoke.mjs"')})[0], [])

    def test_nothing_to_report_says_so(self):
        found, summary = run({'docs/a.md': 'a\n'}, {'docs/a.md': 'b\n'})
        self.assertEqual(found, [])
        self.assertTrue(summary.startswith('No test edits to look at'))


class NeverFails(unittest.TestCase):
    def test_a_broken_run_is_a_warning(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run([sys.executable, str(SCRIPT), 'no-such-base', 'HEAD'], cwd=tmp,
                                    env=ENV, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertIn('::warning title=Test edits::the report failed', result.stdout)



class Review(unittest.TestCase):
    """What the review of #1091 broke or showed missing."""

    def test_ci_runs_the_base_branchs_copy(self):
        # A pull request that edits the script or drops a row still gets the base's report.
        step = next(step for step in yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())
                    ['jobs']['changes']['steps'] if step.get('name') == 'Test edits to look at')
        self.assertEqual(step['timeout-minutes'], 1)
        script = '.github/workflows/scripts/edited_tests.py'
        for base, expected in (('print("base")\n', 'base'), (None, 'head')):
            with self.subTest(base=base), tempfile.TemporaryDirectory() as tmp:
                def git(*args):
                    subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args],
                                   cwd=tmp, env=ENV, check=True, capture_output=True)
                git('init', '-q')
                (Path(tmp) / 'x').write_text('x\n')
                if base:
                    (Path(tmp) / script).parent.mkdir(parents=True)
                    (Path(tmp) / script).write_text(base)
                git('add', '-A')
                git('commit', '-q', '-m', 'base')
                (Path(tmp) / script).parent.mkdir(parents=True, exist_ok=True)
                (Path(tmp) / script).write_text('print("head")\n')
                git('add', '-A')
                git('commit', '-q', '-m', 'head')
                result = subprocess.run(['bash', '-c', step['run']], cwd=tmp, env=ENV,
                                        capture_output=True, text=True, check=True)
                self.assertEqual(result.stdout.strip(), expected)

    def test_the_report_and_its_step_are_runner_configuration(self):
        self.assertEqual(kinds({'.github/workflows/scripts/edited_tests.py': 'a = 1\n'},
                               {'.github/workflows/scripts/edited_tests.py': 'a = 2\n'}),
                         [('runner configuration', '.github/workflows/scripts/edited_tests.py')])
        ci = '      - name: Test edits to look at\n        run: python3 edited_tests.py\n      - run: other\n'
        self.assertEqual(kinds({'.github/workflows/ci.yml': ci},
                               {'.github/workflows/ci.yml': ci.replace('edited_tests.py', 'true')}),
                         [('runner configuration', '.github/workflows/ci.yml')])
        self.assertEqual(kinds({'.github/workflows/ci.yml': ci},
                               {'.github/workflows/ci.yml': ci.replace('other', 'else')}), [])

    def test_bytes_that_are_not_utf8_and_a_lone_carriage_return(self):
        found, _ = run({'web/src/a.test.ts': 'it("one", () => {});\nit("two", () => {});\n',
                        'web/src/b.test.ts': b'it("x", () => {\n  f("\xff\r@@ -1 +1 @@");\n});\n'},
                       {'web/src/a.test.ts': 'it("one", () => {});\n',
                        'web/src/b.test.ts': b'it("x", () => {\n  f("\xfe\r@@ -2 +2 @@");\n});\n'})
        self.assertEqual([(kind, text) for kind, _, _, text in found], [('test deleted', 'two')])
        # A carriage return inside a line splits no line: the numbers stay right.
        line = 'it("one", () => { f("\r@@ -90 +90 @@"); });\n'
        found, _ = run({'web/src/c.test.ts': line + 'it("two", () => {});\n'}, {'web/src/c.test.ts': line})
        self.assertEqual([(kind, line) for kind, _, line, _ in found], [('test deleted', 2)])

    def test_a_crash_is_in_the_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / 'summary.md'
            subprocess.run([sys.executable, str(SCRIPT), 'no-such-base', 'HEAD'], cwd=tmp, check=True,
                           env={**ENV, 'GITHUB_STEP_SUMMARY': str(summary)}, capture_output=True)
            self.assertIn('The report failed', summary.read_text())

    def test_re_indenting_a_test_file_reports_nothing(self):
        # The first 700 lines of web/src/voice/session.test.ts, re-indented as
        # sed 's/^/  /' does: without -w, git pairs the runs so that assertions
        # seem to vanish. Bytes, so that the helper does not dedent them.
        text = (ROOT / 'deploy/compose/tests/fixtures/reindent.test.ts.txt').read_text()
        before = text.encode()
        after = ''.join('  ' + line + '\n' for line in text.splitlines()).encode()
        self.assertEqual(run({'web/src/a.test.ts': before}, {'web/src/a.test.ts': after})[0], [])

    def test_a_large_hunk_takes_linear_time(self):
        lines = ''.join(f'  expect(f({n})).toBe({n});\n' for n in range(12000))
        start = time.monotonic()
        run({'web/src/a.test.ts': 'it("a", () => {\n' + lines + '});\n'},
            {'web/src/a.test.ts': 'it("a", () => {\n' + lines.replace('toBe(', 'toEqual(') + '});\n'})
        self.assertLess(time.monotonic() - start, 10)

    def test_markup_and_column_breaks_stay_text(self):
        before = 'it("a", () => {\n  expect(x).toBe("`<b>|</b>` 50%");\n});\n'
        found, summary = run({'web/src/a.test.ts': before},
                             {'web/src/a.test.ts': 'it("a", () => {\n});\n'})
        self.assertEqual([kind for kind, *_ in found], ['assertion removed'])
        self.assertIn('&lt;b&gt;&#124;&lt;/b&gt;', summary)
        self.assertNotIn('<b>', summary)
        self.assertIn('50%25', run.stdout)

    def test_paths_that_need_escaping_or_quoting(self):
        found, _ = run({'web/src/a,b:c.test.ts': 'it("one", () => {});\nit("two", () => {});\n'},
                       {'web/src/a,b:c.test.ts': 'it("one", () => {});\n'})
        self.assertEqual([(kind, path) for kind, path, *_ in found], [('test deleted', 'web/src/a,b:c.test.ts')])
        self.assertIn('file=web/src/a%2Cb%3Ac.test.ts,', run.stdout)
        quoted = 'web/src/say "hi".test.ts'
        found, _ = run({quoted: 'it("one", () => {});\nit("two", () => {});\n'}, {quoted: 'it("one", () => {});\n'})
        self.assertEqual([(kind, path) for kind, path, *_ in found], [('test deleted', quoted)])

    def test_patterns_inside_strings_and_comments_do_not_count(self):
        before = 'it("a", () => {\n  run();\n});\n'
        for added in ('  const name = "it.skip(";\n', '  // await sleep(5);\n', '  # it.skip(x)\n'):
            with self.subTest(added=added):
                after = before.replace('  run();\n', '  run();\n' + added)
                self.assertEqual(run({'web/src/a.test.ts': before}, {'web/src/a.test.ts': after})[0], [])

    def test_a_deleted_file_and_many_deletions_are_one_row_each(self):
        tests = ''.join(f'it("t{n}", () => {{}});\n' for n in range(5))
        self.assertEqual(kinds({'web/src/a.test.ts': tests}, {'web/src/a.test.ts': None}),
                         [('test file deleted', 'web/src/a.test.ts')])
        self.assertEqual(kinds({'web/src/a.test.ts': tests + 'it("kept", () => {});\n'},
                               {'web/src/a.test.ts': 'it("kept", () => {});\n'}),
                         [('tests deleted', 'web/src/a.test.ts')])

    def test_the_summary_counts_what_the_annotations_leave_out(self):
        before = 'it("a", () => {\n' + ''.join(f'  expect(f({n})).toBe({n});\n' for n in range(12)) + '});\n'
        found, summary = run({'web/src/a.test.ts': before}, {'web/src/a.test.ts': 'it("a", () => {\n});\n'})
        self.assertEqual(len(found), 10)
        self.assertIn('### Test edits to look at (12)', summary)
        self.assertIn('all 12 are here', summary)

    def test_a_cfg_test_item_does_not_make_the_rest_of_the_file_test_code(self):
        code = '#[cfg(test)]\nuse std::fmt;\n\nfn f() -> u8 {\n    1\n}\n'
        looped = code.replace('    1\n', '    loop {\n        return 1;\n    }\n')
        self.assertEqual(run({'api/src/limits.rs': code}, {'api/src/limits.rs': looped})[0], [])
        module = code + '\n#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n    #[test]\n    fn a() {\n        g();\n    }\n}\n'
        self.assertEqual(kinds({'api/src/limits.rs': module},
                               {'api/src/limits.rs': module.replace('        g();\n', '        g();\n        std::thread::sleep(T);\n')}),
                         [('sleep added', 'api/src/limits.rs')])

if __name__ == '__main__':
    unittest.main()
