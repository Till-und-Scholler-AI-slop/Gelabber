"""Cross-language importer controls, using actual pinned libopus packets.

Run after building rtp-source; benchmark-only dependencies stay local.
"""
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location('native_opus_fixture', ROOT / 'opus-fixture.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class NativeAudioImportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        candidates = [Path(os.environ['NATIVE_PEER_BINARY'])] if 'NATIVE_PEER_BINARY' in os.environ else [
            ROOT / 'rtp-source/target/release/gelabber-fixed-native-peer',
            ROOT / 'rtp-source/target/debug/gelabber-fixed-native-peer']
        cls.binary = next((path for path in candidates if path.is_file()), None)
        if cls.binary is None:
            raise unittest.SkipTest('build gelabber-fixed-native-peer or set NATIVE_PEER_BINARY for local cross-language controls')
        cls.folder = tempfile.TemporaryDirectory(prefix='gelabber-native-import-')
        cls.root = Path(cls.folder.name)
        cls.opus = fixture.Opus()
        cls.files = {}
        for kind in ('mic', 'source'):
            path = cls.root / (kind + '.opusbin')
            subprocess.run([sys.executable, str(ROOT / 'opus-fixture.py'), 'build', kind, str(path),
                '--output', str(cls.root / (kind + '.json'))], check=True, capture_output=True)
            cls.files[kind] = path

    @classmethod
    def tearDownClass(cls):
        cls.folder.cleanup()

    def inspect(self, mic=None, source=None):
        return subprocess.run([str(self.binary), '--inspect-audio', str(mic or self.files['mic']),
            str(source or self.files['source'])], capture_output=True, text=True, timeout=15)

    def altered(self, change):
        metadata, packets, _ = fixture.parse(self.files['mic'].read_bytes(), self.opus)
        change(metadata, packets)
        path = self.root / 'altered.opusbin'
        path.write_bytes(fixture.serialize(metadata, packets))
        return path

    def test_real_cross_language_sources_decode_and_regenerate_every_float(self):
        result = self.inspect()
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value['decoder']['sha256'], self.opus.provenance()['library_sha256'])
        for kind in ('mic', 'source'):
            self.assertEqual(value[kind]['sha256'], fixture.digest(self.files[kind].read_bytes()))
            self.assertEqual(value[kind]['metadata']['decode_control']['samples'], 480000)

    def test_sources_cannot_swap_mic_or_source_identity(self):
        self.assertNotEqual(self.inspect(self.files['source'], self.files['mic']).returncode, 0)

    def test_relabelled_packets_with_forged_waveform_and_correlation_are_rejected(self):
        def change(metadata, _packets):
            metadata['pcm'] = fixture.pcm('source')[1]
            metadata['decode_control']['input_correlation_after_codec_lookahead'] = 1
        self.assertNotEqual(self.inspect(self.altered(change)).returncode, 0)

    def test_actual_decoder_and_regenerated_pcm_overrule_metadata_claims(self):
        mutations = [lambda m: m['pcm'].update(sha256='0' * 64),
            lambda m: m['pcm'].update(gain_per_tone=1),
            lambda m: m['decode_control'].update(peak=0),
            lambda m: m['decode_control'].update(float32le_sha256='0' * 64),
            lambda m: m['decode_control'].update(input_correlation_after_codec_lookahead=1),
            lambda m: m['provenance'].update(library_sha256='0' * 64),
            lambda m: m.update(comparison_available=True),
            lambda m: m['encoder'].update(lookahead_samples=0)]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                self.assertNotEqual(self.inspect(self.altered(lambda m, _p: mutate(m))).returncode, 0)

    def test_stereo_and_wrong_duration_do_not_become_mono_twenty_ms_from_labels(self):
        for toc in (0, None):
            def change(metadata, packets):
                packets[0] = bytes([toc if toc is not None else packets[0][0] | 4]) + packets[0][1:]
                metadata['encoded_packets_sha256'] = fixture.digest(b''.join(packets))
            with self.subTest(toc=toc):
                self.assertNotEqual(self.inspect(self.altered(change)).returncode, 0)

    def test_duplicate_policy_and_changed_deadline_are_rejected(self):
        raw = self.files['mic'].read_bytes(); length, = struct.unpack_from('!I', raw, 8)
        header = b'{"pcm":{"kind":"source"},' + raw[13:12 + length]
        changed = raw[:8] + struct.pack('!I', len(header)) + header + raw[12 + length:]
        path = self.root / 'duplicate.opusbin'; path.write_bytes(changed)
        self.assertNotEqual(self.inspect(path).returncode, 0)
        changed = bytearray(raw); struct.pack_into('!Q', changed, 12 + length, 1); path.write_bytes(changed)
        self.assertNotEqual(self.inspect(path).returncode, 0)


if __name__ == '__main__':
    unittest.main()
