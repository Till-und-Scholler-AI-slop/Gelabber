"""The report of test edits to look at (.github/workflows/scripts/edited_tests.py),
run as ci.yml runs it, on scratch repositories."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

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
        found = re.findall(r'^::warning file=([^,]+),line=(\d+),title=Test change: ([^:]+)::(.*)$',
                           result.stdout, re.M)
        return [(kind, path, int(line), text) for path, line, kind, text in found], summary.read_text()


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
        self.assertIn('| `web/src/a.test.ts:2` | test deleted | `two` |', summary)

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


if __name__ == '__main__':
    unittest.main()
