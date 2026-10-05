import copy
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import subprocess
import threading
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from evaluate import summarize, media_stats, same_comparison_browser, main as evaluate_main
from record import process_sample, process_tree


def fixture():
    samples = []
    for index in range(3):
        streams = [{'_endpoint': 'receiver', 'id': 'codec', 'type': 'codec', 'mimeType': 'video/VP8'}]
        for number, kind in enumerate(['audio', 'audio', 'audio', 'video']):
            streams.append({'_endpoint': ('peer-0' if number == 0 else 'peer-1') + '/0', 'id': str(number), 'type': 'inbound-rtp',
                            '_peer': 'peer-0' if number == 0 else 'peer-1',
                            'kind': kind, 'packetsReceived': 10 + index * 100,
                            'packetsLost': 0, 'bytesReceived': index * 10000,
                            'framesDecoded': index * 60, 'frameWidth': 1920, 'frameHeight': 1080})
            streams.append({'_endpoint': 'sender', 'id': str(number), 'type': 'outbound-rtp',
                            'kind': kind, 'bytesSent': index * 10000})
        samples.append({'at': 100000 + index * 1000, 'stats': streams})
    browser = {'backend': 'current', 'failures': [], 'samples': samples, 'peers': 2, 'video': True,
               'join_timing': [{'peer': f'peer-{i}', 'setup_started_at': 1, 'dtls_ready_at': 2,
                                'first_send_rtp_at': 3, 'full_graph_rtp_ready_at': 4} for i in range(2)],
               'post_leave': {'at': 103000, 'engine_stats': {'peers': 0}}}
    server = {'loadgen_returncode': 0, 'idle_samples': [{'rss_bytes': 8 * 1024**2}],
              'post_leave_samples': [{'at': 104, 'rss_bytes': 8 * 1024**2}],
              'samples': [{'at': 100 + index, 'rss_bytes': 16 * 1024**2,
                           'cpu_seconds': index * .1} for index in range(3)]}
    return browser, server


class EvidenceTests(unittest.TestCase):
    def test_pcm_latency_requires_all_unique_edges_markers_and_matching_calibration(self):
        browser, server = fixture()
        browser['input'] = {'pcmLatency': True}
        identity = {'product': 'HeadlessChrome/153.0.8010.12', 'revision': '@fixture', 'sha256': 'a' * 64}
        browser['load_generator'] = {'executed_browser': identity}
        parameters = {'sampleRate': 48000, 'errorBoundMs': 2, 'threshold': .72}
        browser['pcm_calibration'] = {'valid': True, 'failures': [], 'executed_browser': identity,
            'delay_checks': [{'valid': True}] * 4, 'calibration': {'parameters': parameters}}
        edges = [('peer-0', 'peer-1/mic'), ('peer-1', 'peer-0/mic'), ('peer-1', 'peer-0/screen-audio')]
        browser['pcm_latency'] = {'parameters': parameters, 'scope': 'fixture PCM', 'failures': [], 'clipped_frames': 0,
            'sample_clock_seconds': 9, 'wall_clock_seconds': 9,
            'edges': [{'peer': peer, 'source': source, 'expected_markers': 3, 'matches': [
                {'latency_ms': 60 + i, 'score': .9, 'amplitude': .3} for i in range(3)]} for peer, source in edges]}
        for i, sample in enumerate(browser['samples']):
            sample['at'] = 100000 + i * 4000
            for stream in sample['stats']:
                for field in ['bytesReceived', 'bytesSent', 'framesDecoded']:
                    if field in stream: stream[field] *= 4
                if stream['type'] == 'inbound-rtp' and stream['kind'] == 'audio':
                    stream.update(totalSamplesReceived=i * 192000, concealedSamples=0)
        for i, sample in enumerate(server['samples']): sample.update(at=100 + i * 4, cpu_seconds=i * .4)
        server['load_generator_samples'] = [{'at': 100 + i * 4, 'rss_bytes': 100000000, 'cpu_seconds': i * 2} for i in range(3)]
        self.assertTrue(summarize(browser, server)['media_fixture_valid'])
        self.assertEqual(summarize(browser, server)['pcm_latency']['markers'], 9)
        for mutate in [lambda b: b['pcm_latency']['edges'].pop(),
                       lambda b: b['pcm_latency']['edges'][0].update(source='peer-0/mic'),
                       lambda b: b['pcm_latency']['edges'][0]['matches'][0].update(problem='marker not received'),
                       lambda b: b['pcm_latency']['edges'][0]['matches'][0].update(latency_ms=1001),
                       lambda b: b['pcm_calibration'].update(valid=False),
                       lambda b: b['pcm_calibration'].update(executed_browser={**identity, 'sha256': 'b' * 64}),
                       lambda b: b['pcm_latency'].update(clipped_frames=1),
                       lambda b: b['pcm_latency'].update(sample_clock_seconds=8.5),
                       lambda b: b['samples'][-1]['stats'][1].update(concealedSamples=100000)]:
            broken = copy.deepcopy(browser); mutate(broken)
            self.assertFalse(summarize(broken, server)['media_fixture_valid'])
        run = {'executed_browser': identity, 'pcm_latency_enabled': True}
        self.assertTrue(same_comparison_browser([run, copy.deepcopy(run)]))
        self.assertFalse(same_comparison_browser([run, {'executed_browser': identity}]))

    def test_fixed_video_hints_require_actual_sender_receiver_rates_and_browser_provenance(self):
        browser, server = fixture()
        browser['input'] = {'fixedVideoFixture': True, 'videoBitrate': 80000}
        browser['load_generator'] = {'executed_browser': {'product': 'HeadlessChrome/153.0.8010.12', 'revision': '@fixture', 'sha256': 'a' * 64}}
        summary = summarize(browser, server)
        self.assertTrue(summary['media_fixture_valid'])
        self.assertEqual(summary['executed_browser'], browser['load_generator']['executed_browser'])
        broken = copy.deepcopy(browser)
        broken['input']['videoBitrate'] = 4000000
        self.assertFalse(summarize(broken, server)['media_fixture_valid'])
        broken = copy.deepcopy(browser)
        for stream in broken['samples'][-1]['stats']:
            if stream['type'] == 'inbound-rtp' and stream['kind'] == 'video': stream['bytesReceived'] = 10000
        self.assertFalse(summarize(broken, server)['media_fixture_valid'])
        broken = copy.deepcopy(browser)
        del broken['load_generator']
        self.assertFalse(summarize(broken, server)['media_fixture_valid'])

    def test_fixed_video_comparison_rejects_different_executed_browsers_or_source_policies(self):
        run = {'fixed_video_fixture': True, 'executed_browser': {'product': 'HeadlessChrome/153.0.8010.12', 'revision': '@fixture', 'sha256': 'a' * 64}}
        self.assertTrue(same_comparison_browser([run, copy.deepcopy(run)]))
        for field in ['product', 'revision', 'sha256']:
            other = copy.deepcopy(run)
            other['executed_browser'][field] = 'different'
            self.assertFalse(same_comparison_browser([run, other]))
        self.assertFalse(same_comparison_browser([run, {'fixed_video_fixture': False}]))
        self.assertFalse(same_comparison_browser([{'fixed_video_fixture': True}]))

    def test_default_voice_comparison_also_requires_identical_actual_browsers(self):
        run = {'fixed_video_fixture': False, 'executed_browser': {'product': 'HeadlessChrome/153.0.8010.12', 'revision': '@fixture', 'sha256': 'a' * 64}}
        self.assertTrue(same_comparison_browser([run, copy.deepcopy(run)]))
        for field in ['product', 'revision', 'sha256']:
            other = copy.deepcopy(run)
            other['executed_browser'][field] = 'different'
            self.assertFalse(same_comparison_browser([run, other]))
        self.assertFalse(same_comparison_browser([run, {}]))

    def test_interrupted_or_environmentally_disqualified_report_never_allows_comparison(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            report = {'source_revision': 'fixture', 'blocking_gates': [], 'configuration': {
                'matrix': [2], 'engines': ['current', 'mediasoup'], 'runs': 3, 'video': True}, 'runs': []}
            for engine in ['current', 'mediasoup']:
                for number in range(3):
                    browser, server = fixture()
                    browser['backend'] = engine
                    browser['load_generator'] = {'executed_browser': {'product': 'HeadlessChrome/153.0.8010.12', 'revision': '@fixture', 'sha256': 'a' * 64}}
                    if engine == 'mediasoup':
                        for sample in browser['samples']:
                            for stream in sample['stats']:
                                if stream['type'] == 'inbound-rtp': stream['_endpoint'] = stream['_endpoint'].replace('/0', '/recv')
                    folder = directory / f'{engine}-{number}'; folder.mkdir()
                    (folder / 'browser.json').write_text(json.dumps(browser))
                    (folder / 'server.json').write_text(json.dumps(server))
                    report['runs'].append({'engine': engine, 'peers': 2, 'directory': str(folder)})
            for field, reason in [('error', 'KeyboardInterrupt'), ('environment_disqualified', 'network change during run')]:
                content = {**report, field: reason}
                (directory / 'report.json').write_text(json.dumps(content))
                with patch('sys.argv', ['evaluate.py', temporary]), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(evaluate_main(), 1)
                summary = json.loads((directory / 'summary.json').read_text())
                self.assertTrue(all(run['media_fixture_valid'] for run in summary['runs']))
                self.assertEqual(summary['comparison_disqualified'], reason)
                self.assertFalse(any(row['equal_streams_comparison_available'] for row in summary['medians']))

    def test_all_media_edges_and_quality_required_before_resource_comparison(self):
        browser, server = fixture()
        valid = summarize(browser, server)
        self.assertTrue(valid['media_fixture_valid'])
        self.assertAlmostEqual(valid['cpu_cores_mean'], .1)
        broken = copy.deepcopy(browser)
        broken['samples'][-1]['stats'] = [s for s in broken['samples'][-1]['stats']
                                        if not (s['type'] == 'inbound-rtp' and s.get('kind') == 'video')]
        self.assertFalse(summarize(broken, server)['media_fixture_valid'])
        broken = copy.deepcopy(browser)
        broken['samples'][-1]['stats'][7]['frameWidth'] = 640
        self.assertFalse(summarize(broken, server)['media_fixture_valid'])

    def test_probator_is_not_an_extra_video_edge(self):
        browser, server = fixture()
        browser['samples'][-1]['stats'].append({'_endpoint': 'peer-1/0', 'id': 'probator',
            'type': 'inbound-rtp', 'mid': 'probator', 'kind': 'video', 'packetsReceived': 123})
        self.assertTrue(summarize(browser, server)['media_fixture_valid'])

    def test_aggregate_graph_cannot_hide_a_missing_peer_subscription(self):
        browser, server = fixture()
        for stream in browser['samples'][-1]['stats']:
            if stream['type'] == 'inbound-rtp' and stream.get('_peer') == 'peer-1':
                stream['_peer'] = 'peer-0'
        evidence = summarize(browser, server)
        self.assertFalse(evidence['media_fixture_valid'])
        self.assertEqual(evidence['expected_inbound'], evidence['observed_inbound'])

    def test_missing_join_and_leave_evidence_does_not_qualify(self):
        browser, server = fixture()
        browser['join_timing'] = []
        server['post_leave_samples'] = []
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])

    def test_remaining_engine_resources_fail_leave_check(self):
        browser, server = fixture()
        browser['post_leave']['engine_stats']['peers'] = 1
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])

    def test_media_on_wrong_peer_transport_does_not_qualify(self):
        browser, server = fixture()
        browser['samples'][-1]['stats'][1]['_endpoint'] = 'peer-0/unexpected'
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])

    def test_codec_ids_are_scoped_to_the_endpoint(self):
        sample = {'stats': [{'_endpoint': 'a', 'id': 'c', 'type': 'codec', 'mimeType': 'video/rtx'},
                            {'_endpoint': 'b', 'id': 'c', 'type': 'codec', 'mimeType': 'video/VP8'},
                            {'_endpoint': 'a', 'id': 'rtx', 'type': 'inbound-rtp', 'codecId': 'c'},
                            {'_endpoint': 'b', 'id': 'vp8', 'type': 'inbound-rtp', 'codecId': 'c'}]}
        self.assertEqual([s['id'] for s in media_stats(sample, 'inbound-rtp')], ['vp8'])

    def test_advancing_graph_cannot_hide_an_audio_delivery_failure(self):
        browser, server = fixture()
        browser['samples'][-1]['stats'][1]['bytesReceived'] = 10000
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])
        browser, server = fixture()
        browser['samples'][-1]['stats'][1]['packetsLost'] = 30
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])

    def test_failed_or_missing_evidence_cannot_qualify(self):
        browser, server = fixture()
        server['loadgen_returncode'] = 1
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])
        server['loadgen_returncode'] = 0
        server['samples'] = []
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])
        browser['samples'] = []
        self.assertFalse(summarize(browser, server)['media_fixture_valid'])

    def test_linux_resource_sampler_reads_the_actual_owned_process(self):
        sample = process_sample(os.getpid())
        self.assertEqual(sample['pid'], os.getpid())
        self.assertGreater(sample['rss_bytes'], 0)
        self.assertGreaterEqual(sample['cpu_seconds'], 0)
        self.assertIn(os.getpid(), process_tree(os.getpid()))

    def test_process_tree_includes_children_spawned_by_another_thread(self):
        ready, stop, child = threading.Event(), threading.Event(), []
        def spawn():
            process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
            child.append(process)
            ready.set()
            stop.wait(5)
            process.terminate()
            process.wait()
        thread = threading.Thread(target=spawn)
        thread.start()
        try:
            self.assertTrue(ready.wait(5))
            self.assertIn(child[0].pid, process_tree(os.getpid()))
        finally:
            stop.set()
            thread.join()


if __name__ == '__main__':
    unittest.main()
