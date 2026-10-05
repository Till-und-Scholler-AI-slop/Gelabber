import copy
import os
from pathlib import Path
import sys
import subprocess
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from evaluate import summarize, media_stats
from record import process_sample, process_tree


def fixture():
    samples = []
    for index in range(3):
        streams = [{'_endpoint': 'receiver', 'id': 'codec', 'type': 'codec', 'mimeType': 'video/VP8'}]
        for number, kind in enumerate(['audio', 'audio', 'audio', 'video']):
            streams.append({'_endpoint': ('peer-0' if number == 0 else 'peer-1') + '/0', 'id': str(number), 'type': 'inbound-rtp',
                            '_peer': 'peer-0' if number == 0 else 'peer-1',
                            'kind': kind, 'packetsReceived': 10 + index * 100,
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
