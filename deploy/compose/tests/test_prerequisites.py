"""prerequisites: a missing tool skips a test, and fails it in CI."""
import os
import unittest
from unittest import mock

from prerequisites import in_ci, missing, requires

SKIPPED = 'a tool is missing on this machine (with CI set, this fails)'
FAILED = 'a tool is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)'


def run(case, *names):
    result = unittest.TestResult()
    for name in names:
        case(name).run(result)
    return result


class Requires(unittest.TestCase):
    def on_a_test(self, present, ci):
        class Case(unittest.TestCase):
            @requires(present, 'a tool', ci=ci)
            def test_it(self):
                pass
        return run(Case, 'test_it')

    def on_a_class(self, present, ci):
        @requires(present, 'a tool', ci=ci)
        class Case(unittest.TestCase):
            def test_one(self):
                pass

            def test_two(self):
                pass
        return run(Case, 'test_one', 'test_two')

    def test_a_present_tool_runs_the_tests(self):
        for ci in (False, True):
            for result, count in ((self.on_a_test(True, ci), 1), (self.on_a_class(True, ci), 2)):
                self.assertEqual((result.testsRun, result.skipped, result.failures), (count, [], []))

    def test_outside_ci_a_missing_tool_skips(self):
        for result, count in ((self.on_a_test(False, False), 1), (self.on_a_class(False, False), 2)):
            self.assertEqual([reason for _, reason in result.skipped], [SKIPPED] * count)
            self.assertEqual((result.testsRun, result.failures, result.errors), (count, [], []))

    def test_in_ci_a_missing_tool_fails(self):
        for result, count in ((self.on_a_test(False, True), 1), (self.on_a_class(False, True), 2)):
            self.assertEqual((result.testsRun, result.skipped, result.errors), (count, [], []))
            self.assertEqual(len(result.failures), count)
            for _, trace in result.failures:
                self.assertIn(FAILED, trace)

    def test_missing_inside_a_test_skips_or_fails(self):
        with self.assertRaises(unittest.SkipTest) as skipped:
            missing('a tool', ci=False)
        self.assertEqual(str(skipped.exception), SKIPPED)
        with self.assertRaises(AssertionError) as failed:
            missing('a tool', ci=True)
        self.assertEqual(str(failed.exception), FAILED)


class InCi(unittest.TestCase):
    def test_ci_counts_unless_unset_empty_0_or_false(self):
        for value, expected in (('true', True), ('1', True), ('yes', True), ('', False),
                                ('0', False), ('false', False), ('False', False), (None, False)):
            with self.subTest(value=value):
                environ = {k: v for k, v in os.environ.items() if k != 'CI'}
                if value is not None:
                    environ['CI'] = value
                with mock.patch.dict(os.environ, environ, clear=True):
                    self.assertIs(in_ci(), expected)


if __name__ == '__main__':
    unittest.main()
