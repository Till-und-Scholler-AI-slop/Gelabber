import importlib.util
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parents[1]))
spec = importlib.util.spec_from_file_location('fixed_video', Path(__file__).parents[1] / 'run-fixed-video.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
sys.path.pop(0)


class ResourceEvidence(unittest.TestCase):
    def fixture(self):
        sample = {'root_pid': 123, 'processes': [{'pid': 123, 'rss_bytes': 1000, 'cpu_seconds': 1.0}], 'rss_bytes': 1000, 'cpu_seconds': 1.0}
        return {'engine': 'current', 'idle_sample': sample, 'samples': [sample, sample],
                'load_generator_samples': [sample, sample], 'post_leave_samples': [sample, sample],
                'infrastructure_samples': [sample, sample], 'monitoring_errors': []}

    def test_complete_separate_backend_generator_infra_and_leave_measurements(self):
        self.assertEqual(runner.resource_failures(self.fixture()), [])

    def test_monitor_failure_and_missing_pid_cpu_rss_are_invalid(self):
        for field in ['samples', 'load_generator_samples', 'infrastructure_samples', 'post_leave_samples']:
            result = self.fixture(); result[field] = []
            self.assertTrue(runner.resource_failures(result), field)
        for change in [{'processes': []}, {'rss_bytes': 0}, {'cpu_seconds': float('nan')}]:
            result = self.fixture(); result['idle_sample'] = {**result['idle_sample'], **change}
            self.assertTrue(runner.resource_failures(result), change)
        result = self.fixture(); result['monitoring_errors'] = ['PID vanished']
        self.assertIn('PID vanished', runner.resource_failures(result))
        for processes in [[{}], [{'pid': 999, 'rss_bytes': 1000, 'cpu_seconds': 1}], [{'pid': 123, 'rss_bytes': 1000}]]:
            result = self.fixture(); result['samples'] = [{**result['samples'][0], 'processes': processes}] * 2
            self.assertTrue(runner.resource_failures(result), processes)
