"""prerequisites: a missing tool skips a test, and fails it in CI."""
import unittest

from prerequisites import missing, requires


class Requires(unittest.TestCase):
    def outcome(self, present, ci):
        class Case(unittest.TestCase):
            @requires(present, 'a tool', ci=ci)
            def test_it(self):
                pass
        result = unittest.TestResult()
        Case('test_it').run(result)
        return result

    def test_a_present_tool_runs_the_test(self):
        for ci in (False, True):
            result = self.outcome(True, ci)
            self.assertEqual((result.testsRun, result.skipped, result.failures), (1, [], []))

    def test_outside_ci_a_missing_tool_skips(self):
        result = self.outcome(False, False)
        self.assertEqual([reason for _, reason in result.skipped],
                         ['a tool is missing on this machine (with CI set, this fails)'])
        self.assertEqual(result.failures, [])

    def test_in_ci_a_missing_tool_fails(self):
        result = self.outcome(False, True)
        self.assertEqual(result.skipped, [])
        self.assertEqual(len(result.failures), 1)
        self.assertIn('a tool is missing; CI must provide it', result.failures[0][1])

    def test_missing_inside_a_test_skips_or_fails(self):
        with self.assertRaisesRegex(unittest.SkipTest, 'a tool is missing on this machine'):
            missing('a tool', ci=False)
        with self.assertRaisesRegex(AssertionError, 'a tool is missing; CI must provide it'):
            missing('a tool', ci=True)


if __name__ == '__main__':
    unittest.main()
