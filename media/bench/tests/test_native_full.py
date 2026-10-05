"""No Docker mutations: collector phase and failure controls."""
import importlib.util
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('native_full_runner', ROOT / 'run-native-full.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def sample(at, cpu, phase='measurement', valid=True):
    return {'at': at, 'cpu_seconds': cpu, 'rss_bytes': 1024, 'phase': phase, 'valid': valid}


class NativeFullResourceTests(unittest.TestCase):
    def test_post_leave_and_expected_cleanup_exit_are_not_active_zeroes(self):
        result = runner.resources([sample(0, 0), sample(1, .5),
            {'at': 2, 'phase': 'cleanup', 'valid': False, 'error': 'child ended'}, sample(3, .7, 'post_leave')], minimum_span=1)
        self.assertTrue(result['measurement_valid'])
        self.assertEqual(result['cpu_core_equivalents'], [.5])
        self.assertEqual(result['measurement_samples'], 2)

    def test_missing_pid_or_sample_in_measurement_invalidates_resources(self):
        for bad in [dict(at=.5, phase='measurement', valid=False, error='PID disappeared'), sample(.5, .25, valid='false')]:
            self.assertFalse(runner.resources([sample(0, 0), bad, sample(1, .5)])['measurement_valid'])

    def test_short_window_reversed_clock_and_cpu_reset_do_not_pass(self):
        for values in [[sample(0, 0), sample(.5, .2)], [sample(0, 0), sample(0, .2), sample(2, .5)],
                       [sample(0, .5), sample(2, .2)]]:
            self.assertFalse(runner.resources(values, minimum_span=1)['measurement_valid'])

    def test_post_leave_requires_three_actual_seconds(self):
        self.assertFalse(runner.resources([sample(0, 0, 'post_leave'), sample(2.5, .1, 'post_leave')], 'post_leave', 3)['measurement_valid'])
        result = runner.resources([sample(0, 0, 'post_leave'), sample(3.5, .1, 'post_leave')], 'post_leave', 3)
        self.assertTrue(result['measurement_valid'])
        self.assertEqual(result['actual_span_seconds'], 3.5)


if __name__ == '__main__':
    unittest.main()
