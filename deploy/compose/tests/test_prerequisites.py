"""prerequisites: a missing tool skips a test, and fails it in CI."""
import os
import unittest
from unittest import mock

from prerequisites import in_ci, missing, requires

SKIPPED = 'a tool is missing on this machine (with CI set, this fails)'
FAILED = 'a tool is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)'


def run(case, *names):
    """Runs the class as unittest does (with setUpClass), or only `names`."""
    result = unittest.TestResult()
    if names:
        for name in names:
            case(name).run(result)
    else:
        unittest.TestLoader().loadTestsFromTestCase(case).run(result)
    return result


def traces(result):
    return ''.join(trace for _, trace in result.failures + result.errors)


class Requires(unittest.TestCase):
    def on_a_test(self, present, ci):
        class Case(unittest.TestCase):
            @requires(present, 'a tool', ci=ci)
            def test_it(self):
                pass
        return run(Case, 'test_it')

    def on_a_class(self, present, ci, calls=None):
        """A class with its own setUpClass and setUp, and an inherited test."""
        calls = [] if calls is None else calls

        class Base(unittest.TestCase):
            def test_inherited(self):
                calls.append('test_inherited')

        @requires(present, 'a tool', ci=ci)
        class Case(Base):
            @classmethod
            def setUpClass(cls):
                calls.append('setUpClass')

            def setUp(self):
                calls.append('setUp')

            def test_own(self):
                calls.append('test_own')
        return Case

    def test_a_present_tool_runs_the_tests(self):
        for ci in (False, True):
            result = self.on_a_test(True, ci)
            self.assertEqual((result.testsRun, result.skipped, result.failures), (1, [], []))
            calls = []
            result = run(self.on_a_class(True, ci, calls))
            self.assertTrue(result.wasSuccessful())
            self.assertEqual(sorted(calls), ['setUp', 'setUp', 'setUpClass', 'test_inherited', 'test_own'])

    def test_outside_ci_a_missing_tool_skips(self):
        result = self.on_a_test(False, False)
        self.assertEqual([reason for _, reason in result.skipped], [SKIPPED])
        self.assertEqual((result.failures, result.errors), ([], []))
        calls = []
        result = run(self.on_a_class(False, False, calls))
        self.assertEqual([reason for _, reason in result.skipped], [SKIPPED] * 2)
        self.assertEqual((result.failures, result.errors, calls), ([], [], []))

    def test_in_ci_a_missing_tool_fails(self):
        result = self.on_a_test(False, True)
        self.assertEqual((result.testsRun, result.skipped, result.errors), (1, [], []))
        self.assertEqual(len(result.failures), 1)
        self.assertIn(FAILED, traces(result))
        calls = []
        result = run(self.on_a_class(False, True, calls))
        self.assertFalse(result.wasSuccessful())
        self.assertEqual((result.skipped, calls), ([], []))
        self.assertIn(FAILED, traces(result))

    def test_in_ci_an_inherited_test_fails_too(self):
        calls = []
        result = run(self.on_a_class(False, True, calls), 'test_inherited', 'test_own')
        self.assertEqual(len(result.failures), 2)
        self.assertNotIn('test_inherited', calls)

    def test_on_set_up_class_it_skips_or_fails_the_class(self):
        for ci in (False, True):
            calls = []

            class Case(unittest.TestCase):
                @requires(False, 'a tool', ci=ci)
                @classmethod
                def setUpClass(cls):
                    calls.append('setUpClass')

                def test_it(self):
                    calls.append('test_it')
            result = run(Case)
            self.assertEqual(calls, [])
            if ci:
                self.assertIn(FAILED, traces(result))
            else:
                self.assertEqual([reason for _, reason in result.skipped], [SKIPPED])
                self.assertEqual(result.errors, [])

    def test_missing_inside_a_test_skips_or_fails(self):
        with self.assertRaises(unittest.SkipTest) as skipped:
            missing('a tool', ci=False)
        self.assertEqual(str(skipped.exception), SKIPPED)
        with self.assertRaises(AssertionError) as failed:
            missing('a tool', ci=True)
        self.assertEqual(str(failed.exception), FAILED)


class InCi(unittest.TestCase):
    def test_ci_counts_unless_unset_empty_0_or_false(self):
        for value, expected in (('true', True), ('TRUE', True), ('1', True), ('yes', True),
                                ('', False), ('0', False), ('false', False), ('False', False),
                                (' false ', False), (None, False)):
            with self.subTest(value=value):
                environ = {k: v for k, v in os.environ.items() if k != 'CI'}
                if value is not None:
                    environ['CI'] = value
                with mock.patch.dict(os.environ, environ, clear=True):
                    self.assertIs(in_ci(), expected)


if __name__ == '__main__':
    unittest.main()
