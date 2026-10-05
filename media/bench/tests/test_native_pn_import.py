"""Actual cross-language finite PN import; no sockets or clock acceptance."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location('native_pn_fixture', ROOT / 'pn-opus-fixture.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
NODE = Path(os.environ.get('GELABBER_PN_NODE', 'node'))
RUN = '6427c31e-419d-4fc8-b6e3-a182fe496cab'


class NativePnImportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ.get('NATIVE_PEER_BINARY', str(ROOT / 'rtp-source/target/release/gelabber-fixed-native-peer')))
        if not cls.binary.is_file():
            raise unittest.SkipTest('build the native binary for opt-in local cross-language checks')
        cls.folder = tempfile.TemporaryDirectory(prefix='gelabber-native-pn-import-')
        cls.root = Path(cls.folder.name)
        opus = fixture.Opus(os.environ.get('GELABBER_PN_OPUS_LIBRARY'))
        cls.loaded, cls.files = {}, {}
        for kind in ('mic', 'source'):
            data, _ = fixture.build(kind, 20, RUN, opus, NODE)
            length, = struct.unpack_from('!I', data, 8)
            metadata = json.loads(data[12:12 + length])
            packets = [data[12 + length + i * 332 + 12:12 + length + (i + 1) * 332] for i in range(1050)]
            cls.loaded[kind] = metadata, packets
            cls.files[kind] = cls.root / (kind + '.opusbin')
            cls.files[kind].write_bytes(data)

    @classmethod
    def tearDownClass(cls):
        cls.folder.cleanup()

    def inspect(self, mic=None, source=None):
        return subprocess.run([str(self.binary), '--inspect-audio', str(mic or self.files['mic']), str(source or self.files['source'])],
                              capture_output=True, text=True, timeout=20)

    def altered(self, kind, change):
        metadata, packets = copy.deepcopy(self.loaded[kind])
        change(metadata, packets)
        path = self.root / ('changed-' + kind + '.opusbin')
        path.write_bytes(fixture.serialize(metadata, packets))
        return path

    def test_actual_full_source_decode_marker_and_explicit_tail_import(self):
        result = self.inspect()
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        for kind in ('mic', 'source'):
            self.assertEqual(value[kind]['metadata']['packets'], 1050)
            self.assertEqual(value[kind]['metadata']['marker_control']['markers'], 10)
            self.assertEqual(value[kind]['metadata']['duration_seconds'], 21)
            self.assertEqual(value[kind]['metadata']['tail_samples'], 48000)

    def test_source_identity_role_codebook_and_whole_pair_are_bound(self):
        self.assertNotEqual(self.inspect(self.files['source'], self.files['mic']).returncode, 0)
        def change(metadata, _packets):
            run = '82bce9e4-5f7f-4586-978e-24a49a7067d4'
            metadata['run_id'] = run
            metadata['pn'] = fixture.codebook('source', 20, run)
        self.assertNotEqual(self.inspect(source=self.altered('source', change)).returncode, 0)
        for change in [lambda m: m['pn'].update(source_uid=64), lambda m: m['pn'].update(ssrc=1),
                       lambda m: m['pn']['markers'].pop(), lambda m: m['pn']['shared_run']['sources'].pop()]:
            self.assertNotEqual(self.inspect(self.altered('mic', lambda m, _p: change(m))).returncode, 0)

    def test_metadata_cannot_forge_actual_pcm_decoder_marker_or_frozen_policy(self):
        for change in [lambda m: m['pcm'].update(gain_per_tone=.1), lambda m: m['pcm'].update(sha256='0' * 64),
                       lambda m: m['decode_control'].update(float32le_sha256='0' * 64),
                       lambda m: m['marker_control']['checks'][0].update(actual_decoded_sample_ordinal=48000),
                       lambda m: m['marker_control']['checks'][0].update(score=1),
                       lambda m: m['marker_control'].update(executed_node={'version': 'v0.0.0', 'sha256': '0' * 64}),
                       lambda m: m['marker_control']['executed_node'].update(version='v0.0.0'),
                       lambda m: m['marker_control']['executed_node'].update(sha256='0' * 64),
                       lambda m: m['provenance'].update(pcm_kernel_sha256='0' * 64),
                       lambda m: m.update(comparison_available=True), lambda m: m.update(pcm_latency_calibrated=True),
                       lambda m: m.update(tail_samples=0), lambda m: m.update(duration_seconds=20)]:
            self.assertNotEqual(self.inspect(self.altered('mic', lambda m, _p: change(m))).returncode, 0)

    def test_missing_tail_packet_wrong_schedule_and_duplicate_keys_are_rejected(self):
        self.assertNotEqual(self.inspect(self.altered('mic', lambda _m, p: p.pop())).returncode, 0)
        raw = self.files['mic'].read_bytes(); length, = struct.unpack_from('!I', raw, 8)
        for duplicate in (False, True):
            if duplicate:
                header = b'{"tail_samples":0,' + raw[13:12 + length]
                changed = raw[:8] + struct.pack('!I', len(header)) + header + raw[12 + length:]
            else:
                changed = bytearray(raw); struct.pack_into('!Q', changed, 12 + length, 1)
            path = self.root / 'changed-raw.opusbin'; path.write_bytes(changed)
            self.assertNotEqual(self.inspect(path).returncode, 0)


if __name__ == '__main__':
    unittest.main()
