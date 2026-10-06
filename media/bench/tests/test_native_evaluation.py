"""Synthetic evaluator contracts only; no browser, Docker or media acceptance."""
import copy
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT))
from evaluate import cgroup_throttling, native_source_comparison, summarize_native, main
spec = importlib.util.spec_from_file_location('native_resources_evaluation', ROOT / 'run-native-full.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def cpu_sample(at, periods, throttled=0, usec=0):
    return {'at': at, 'phase': 'measurement', 'valid': True, 'cpu_seconds': at * .1, 'rss_bytes': 1024,
            'cgroup_path': '/owned-fixture', 'cgroup_cpu_stat': f'usage_usec {at * 100000}\nnr_periods {periods}\nnr_throttled {throttled}\nthrottled_usec {usec}\n'}


def browser_fixture(engine='current'):
    edges = [{'receiver': receiver, 'source': source, 'kind': kind} for receiver, source, kind in [
        ('peer-0', 'peer-1/mic', 'audio'), ('peer-1', 'peer-0/mic', 'audio'),
        ('peer-1', 'peer-0/screen-audio', 'audio'), ('peer-1', 'peer-0/video', 'video')]]
    quality = []
    for edge in edges:
        native = edge['receiver'] == 'peer-0'
        fields = ['packets_received', 'payload_bytes_received', 'decoded_samples', 'sequence_gaps', 'timestamp_gaps', 'reordered_or_duplicate_packets', 'decode_errors'] if native else [
            'packetsReceived', 'bytesReceived', 'packetsLost', 'packetsDiscarded', 'jitterBufferDelay', 'jitterBufferTargetDelay', 'jitterBufferMinimumDelay', 'jitterBufferEmittedCount'] + ([
            'totalSamplesReceived', 'concealedSamples', 'silentConcealedSamples', 'concealmentEvents', 'insertedSamplesForDeceleration', 'removedSamplesForAcceleration'] if edge['kind'] == 'audio' else [
            'framesDecoded', 'framesDropped', 'freezeCount', 'pauseCount', 'totalFreezesDuration', 'totalPausesDuration'])
        quality.append({**edge, 'receiver_implementation': 'native' if native else 'browser',
            'complete': True, 'unavailable': [], 'counters': {field: {'delta': 0, 'unavailable': None} for field in fields},
            'sample_rate': 48000 if edge['kind'] == 'audio' else None, 'decoded_fps': 60 if edge['kind'] == 'video' else None,
            'measured_seconds': 20, 'jitter_buffer_mean_seconds': None if native else {'actual': .02, 'target': .02, 'minimum': .015},
            'packet_stalls': [], 'decoder_stalls': []})
        row = quality[-1]
        for field in (['packets_received', 'payload_bytes_received', 'decoded_samples'] if native else ['packetsReceived', 'bytesReceived', 'totalSamplesReceived'] if edge['kind'] == 'audio' else ['packetsReceived', 'bytesReceived', 'framesDecoded']):
            row['counters'][field]['delta'] = 960000 if field in ['decoded_samples', 'totalSamplesReceived'] else 1200 if field == 'framesDecoded' else 1000
        if not native:
            emitted = 960000 if edge['kind'] == 'audio' else 1200
            row['counters']['jitterBufferEmittedCount']['delta'] = emitted
            for field, value in [('jitterBufferDelay', .02), ('jitterBufferTargetDelay', .02), ('jitterBufferMinimumDelay', .015)]:
                row['counters'][field]['delta'] = emitted * value
            if edge['kind'] == 'audio': row['nonconcealed_sample_rate'] = 48000
    return {'engine': engine, 'topology': {'voice_participants': 2, 'participants': [
        {'peer': 'peer-0', 'implementation': 'native'}, {'peer': 'peer-1', 'implementation': 'browser'}], 'edges': edges},
        'plan': {'seconds': 20, 'warmup': 30, 'source_seconds': 120, 'source_policy': 'same frozen source schedule'},
        'collector': {'node_sha256': 'a' * 64}, 'executed_browser': {'product': 'Chrome/153', 'revision': '@test', 'sha256': 'b' * 64},
        'instrument': {**{name: {'archive_sha256': char * 64} for name, char in [('mic', 'a'), ('source', 'b'), ('video', 'c')]},
            'decoder': {'libopus_sha256': 'd' * 64, 'version': '1.6.1'}},
        'graph': {'valid': True, 'measurement_policy': {'engine': engine, 'requestedSeconds': 20}, 'source_graph': {'valid': True, 'failures': []}, 'quality': {'complete': True, 'edges': quality},
            'senders': [{'source': source, 'bitrate_bps': 4000000 if source.endswith('/video') else 128000} for source in ['peer-0/mic', 'peer-1/mic', 'peer-0/screen-audio', 'peer-0/video']]},
        'full_graph_streams_valid': True, 'failures': []}


class NativeEvaluationTests(unittest.TestCase):
    def test_raw_cgroup_throttling_is_independent_of_below_quota_process_mean(self):
        samples = [cpu_sample(0, 100, 10, 10000), cpu_sample(1, 110, 12, 20000), cpu_sample(2, 120, 15, 60000)]
        result = cgroup_throttling(samples)
        self.assertTrue(result['available'])
        self.assertEqual(result['delta'], {'nr_periods': 20, 'nr_throttled': 5, 'throttled_usec': 50000})
        self.assertEqual(result['throttled_seconds'], .05)
        self.assertEqual(result['throttled_period_fraction'], .25)
        self.assertTrue(result['observed_throttling'])
        resource = runner.resources(samples, minimum_span=2)
        self.assertEqual(resource['cpu_core_equivalents'], [.1, .1])
        self.assertTrue(resource['cgroup_throttling']['observed_throttling'])

    def test_missing_reset_clock_and_identity_failures_are_unavailable_not_zero(self):
        for mutate in [lambda values: values[1].pop('cgroup_cpu_stat'),
                       lambda values: values[1].update(cgroup_cpu_stat='nr_periods 0\nnr_throttled 0\nthrottled_usec 0\n'),
                       lambda values: values[1].update(at=0),
                       lambda values: values[1].update(cgroup_path='/different-owned-group'),
                       lambda values: values[1].update(cgroup_cpu_stat='nr_periods 2\nnr_periods 3\nnr_throttled 0\nthrottled_usec 0\n'),
                       lambda values: values[1].update(cgroup_cpu_stat='nr_periods -2\nnr_throttled 0\nthrottled_usec 0\n')]:
            values = [cpu_sample(0, 10), cpu_sample(1, 20), cpu_sample(2, 30)]; mutate(values)
            result = cgroup_throttling(values)
            self.assertFalse(result['available']); self.assertIsNone(result['delta'])
        self.assertFalse(cgroup_throttling([])['available'])
        self.assertFalse(cgroup_throttling([cpu_sample(0, 0)])['available'])

    def test_sampler_reads_actual_pid_group_and_preserves_baseline_end(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); proc = root / 'proc'; groups = root / 'groups'
            (proc / '123').mkdir(parents=True); (groups / 'fixture').mkdir(parents=True)
            (proc / '123' / 'cgroup').write_text('0::/fixture\n')
            (groups / 'fixture' / 'cpu.stat').write_text(cpu_sample(0, 10)['cgroup_cpu_stat'])
            before = runner.cgroup_cpu_sample(123, proc, groups)
            (groups / 'fixture' / 'cpu.stat').write_text(cpu_sample(1, 20, 2, 10000)['cgroup_cpu_stat'])
            after = runner.cgroup_cpu_sample(123, proc, groups)
            self.assertEqual(before['cgroup_path'], '/fixture')
            result = cgroup_throttling([{'at': 0, **before}, {'at': 1, **after}])
            self.assertEqual(result['delta']['nr_throttled'], 2)
            (proc / '123' / 'cgroup').write_text('0::/../../outside\n')
            with self.assertRaises(ValueError): runner.cgroup_cpu_sample(123, proc, groups)
            (proc / '123' / 'cgroup').write_text('1:cpu:/fixture\n')
            with self.assertRaises(ValueError): runner.cgroup_cpu_sample(123, proc, groups)

    def test_quality_does_not_turn_historical_nonzero_plc_failure_into_a_pass(self):
        browser = browser_fixture()
        browser['graph']['valid'] = browser['full_graph_streams_valid'] = False
        browser['failures'] = ['strict pilot observed 514 concealed samples']
        browser['graph']['quality']['edges'][1]['counters']['concealedSamples']['delta'] = 514
        browser['graph']['quality']['edges'][1]['nonconcealed_sample_rate'] = (960000 - 514) / 20
        result = summarize_native(browser, {'samples': [cpu_sample(0, 0), cpu_sample(1, 10)]})
        self.assertFalse(result['strict_pilot_valid']); self.assertTrue(result['source_graph_valid'])
        self.assertTrue(result['quality_data_complete']); self.assertTrue(result['measurement_comparable'])
        self.assertEqual(result['strict_pilot_failures'], browser['failures'])
        self.assertEqual(result['measured_quality']['edges'][1]['counters']['concealedSamples']['delta'], 514)
        self.assertFalse(result['comparison_available']); self.assertFalse(result['acceptance'])

    def test_complete_claim_cannot_hide_missing_edges_counters_or_proposed_false_flags(self):
        for mutate in [lambda b: b['graph']['quality']['edges'].pop(),
                       lambda b: b['graph']['quality']['edges'][1]['counters'].pop('silentConcealedSamples'),
                       lambda b: b['graph']['quality']['edges'][1]['counters']['concealedSamples'].update(delta=True),
                       lambda b: b['graph']['quality']['edges'][1].update(complete='true'),
                       lambda b: b['graph']['quality']['edges'][1].update(receiver_implementation='native'),
                       lambda b: b['graph']['quality']['edges'][1].update(sample_rate=24000),
                       lambda b: b['graph']['quality']['edges'][1]['jitter_buffer_mean_seconds'].update(actual=.001),
                       lambda b: b['topology']['edges'].append(copy.deepcopy(b['topology']['edges'][0]))]:
            browser = browser_fixture(); mutate(browser)
            result = summarize_native(browser, {})
            self.assertFalse(result['quality_data_complete']); self.assertFalse(result['measurement_comparable'])
            self.assertFalse(result['backend_throttling']['available']); self.assertFalse(result['comparison_available'])

    def test_source_comparison_rejects_unequal_archives_schedule_inventory_rates_and_runtime(self):
        baseline, candidate = browser_fixture(), browser_fixture('janus')
        self.assertTrue(native_source_comparison([baseline, candidate])['source_graph_comparison_available'])
        self.assertFalse(native_source_comparison([baseline, candidate])['comparison_available'])
        mutations = [lambda b: b['instrument']['mic'].update(archive_sha256='f' * 64),
            lambda b: b['plan'].update(warmup=10), lambda b: b['plan'].pop('source_policy'),
            lambda b: b['graph']['senders'].pop(), lambda b: b['graph']['senders'][0].update(bitrate_bps=64000),
            lambda b: b['graph']['source_graph'].update(valid=False), lambda b: b['collector'].update(node_sha256='f' * 64),
            lambda b: b['graph']['measurement_policy'].update(requestedSeconds=10),
            lambda b: b['executed_browser'].update(sha256='f' * 64), lambda b: b['topology'].update(voice_participants=8)]
        for mutate in mutations:
            broken = copy.deepcopy(candidate); mutate(broken)
            self.assertFalse(native_source_comparison([baseline, broken])['source_graph_comparison_available'])
        self.assertFalse(native_source_comparison([candidate])['source_graph_comparison_available'])
        self.assertFalse(native_source_comparison([candidate, browser_fixture('mediasoup')])['source_graph_comparison_available'])
        self.assertFalse(native_source_comparison([baseline, browser_fixture('unknown')])['source_graph_comparison_available'])
        self.assertFalse(native_source_comparison([baseline, candidate, copy.deepcopy(candidate)])['source_graph_comparison_available'])
        spoofed = copy.deepcopy(baseline); spoofed['engine'] = 'mediasoup'
        self.assertFalse(native_source_comparison([baseline, spoofed])['source_graph_comparison_available'])

    def test_cli_summarizes_native_pilots_without_performance_or_latency_approval(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            report = {'peers': 2, 'backend_images': {'current': {}, 'janus': {}}, 'runs': []}
            for engine in ['current', 'janus']:
                folder = directory / engine; folder.mkdir()
                browser = browser_fixture(engine)
                (folder / 'browser.json').write_text(json.dumps(browser))
                (folder / 'server.json').write_text(json.dumps({'samples': [cpu_sample(0, 0), cpu_sample(1, 10)]}))
                report['runs'].append({'engine': engine})
            (directory / 'report.json').write_text(json.dumps(report))
            with patch('sys.argv', ['evaluate.py', temporary]), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(), 0)
            output = json.loads((directory / 'summary.json').read_text())
            self.assertTrue(output['source_comparison']['source_graph_comparison_available'])
            self.assertFalse(output['comparison_available']); self.assertFalse(output['acceptance'])
            self.assertTrue(all(not run['comparison_available'] for run in output['runs']))
            report['environment_disqualified'] = 'host network changed'
            (directory / 'report.json').write_text(json.dumps(report))
            with patch('sys.argv', ['evaluate.py', temporary]), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(), 1)
            output = json.loads((directory / 'summary.json').read_text())
            self.assertFalse(output['source_comparison']['source_graph_comparison_available'])


if __name__ == '__main__':
    unittest.main()
