import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / 'opus-fixture.py'
spec = importlib.util.spec_from_file_location('opus_fixture', SCRIPT)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class OpusFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.opus = fixture.Opus()
        cls.folder = tempfile.TemporaryDirectory(prefix='gelabber-opus-control-')
        cls.root = Path(cls.folder.name)
        cls.files = {}
        for kind in ('mic', 'source'):
            archive = cls.root / (kind + '.opusbin')
            subprocess.run([sys.executable, str(SCRIPT), 'build', kind, str(archive),
                            '--output', str(cls.root / (kind + '.json'))], check=True, capture_output=True)
            cls.files[kind] = archive.read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.folder.cleanup()

    def altered(self, change):
        metadata, packets, _ = fixture.parse(self.files['mic'], self.opus)
        change(metadata, packets)
        return fixture.serialize(metadata, packets)

    def test_two_distinct_real_sources_have_fixed_rate_and_decode_every_packet(self):
        hashes = []
        for kind, data in self.files.items():
            metadata, packets, decoded = fixture.parse(data, self.opus)
            self.assertEqual(len(packets), 500)
            self.assertEqual({len(packet) for packet in packets}, {320})
            self.assertEqual(len(decoded), 480_000)
            self.assertEqual(metadata['pcm']['kind'], kind)
            self.assertGreater(metadata['decode_control']['input_correlation_after_codec_lookahead'], .98)
            hashes.append(metadata['encoded_packets_sha256'])
        self.assertNotEqual(*hashes)

    def test_truncation_extra_packets_and_wrong_magic_do_not_qualify(self):
        data = self.files['mic']
        for changed in (data[:8], b'BADMAGIC' + data[8:], data[:-1], data + data[-332:]):
            with self.subTest(length=len(changed)), self.assertRaises(ValueError):
                fixture.parse(changed, self.opus)

    def test_packet_mutation_is_detected_without_trusting_metadata_claims(self):
        data = bytearray(self.files['mic']); data[-1] ^= 1
        with self.assertRaises(ValueError): fixture.parse(bytes(data), self.opus)

    def test_schedule_and_actual_payload_length_are_checked(self):
        for field, value in ((0, 1), (8, 319), (8, 321)):
            data = bytearray(self.files['mic']); header, = struct.unpack_from('!I', data, 8)
            struct.pack_into('!Q' if field == 0 else '!I', data, 12 + header + field, value)
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                fixture.parse(bytes(data), self.opus)

    def test_wrong_duration_is_rejected_even_with_a_matching_payload_hash(self):
        def mutate(metadata, packets):
            packets[0] = bytes([0]) + packets[0][1:]  # SILK 10ms, not the required 20ms.
            metadata['encoded_packets_sha256'] = fixture.digest(b''.join(packets))
        with self.assertRaises(ValueError): fixture.parse(self.altered(mutate), self.opus)

    def test_actual_stereo_toc_is_rejected_even_with_mono_labels_and_matching_hash(self):
        def mutate(metadata, packets):
            packets[:] = [bytes([packet[0] | 4]) + packet[1:] for packet in packets]
            metadata['encoded_packets_sha256'] = fixture.digest(b''.join(packets))
        with self.assertRaises(ValueError): fixture.parse(self.altered(mutate), self.opus)

    def test_mic_bytes_cannot_be_relabelled_as_the_other_source_waveform(self):
        def mutate(metadata, _packets):
            metadata['pcm'] = fixture.pcm('source')[1]
            metadata['decode_control']['input_correlation_after_codec_lookahead'] = 1
        with self.assertRaises(ValueError): fixture.parse(self.altered(mutate), self.opus)

    def test_pcm_and_decoder_claims_are_verified_against_actual_samples(self):
        mutations = [lambda m: m['pcm'].update(sha256='0' * 64),
                     lambda m: m['pcm'].update(gain_per_tone=1),
                     lambda m: m['decode_control'].update(float32le_sha256='0' * 64),
                     lambda m: m['decode_control'].update(input_correlation_after_codec_lookahead=1),
                     lambda m: m['encoder'].update(lookahead_samples=0)]
        for mutate in mutations:
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                fixture.parse(self.altered(lambda metadata, _packets: mutate(metadata)), self.opus)

    def test_wrong_sampleclock_rate_channel_count_and_encoder_flags_are_rejected(self):
        for field, value in (('sample_rate_hz', 44_100), ('channels', 2), ('packets', 499),
                             ('payload_bitrate_bps', 64_000), ('encoded_bytes', 0)):
            data = self.altered(lambda metadata, _packets: metadata.update({field: value}))
            with self.subTest(field=field), self.assertRaises(ValueError): fixture.parse(data, self.opus)
        data = self.altered(lambda metadata, _packets: metadata['encoder']['settings_readback'].update(dtx=1))
        with self.assertRaises(ValueError): fixture.parse(data, self.opus)

    def test_duplicate_policy_keys_are_rejected(self):
        data = self.files['mic']; length, = struct.unpack_from('!I', data, 8)
        header = b'{"channels":2,' + data[13:12 + length]
        changed = data[:8] + struct.pack('!I', len(header)) + header + data[12 + length:]
        with self.assertRaises(ValueError): fixture.parse(changed, self.opus)

    def test_fresh_outputs_and_no_latency_claim(self):
        destination = self.root / 'mic.opusbin'; original = destination.read_bytes()
        refused = subprocess.run([sys.executable, str(SCRIPT), 'build', 'mic', str(destination),
                                  '--output', str(self.root / 'refused.json')], capture_output=True)
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(destination.read_bytes(), original)
        self.assertFalse((self.root / 'refused.json').exists())
        for kind in self.files:
            metadata = json.loads((self.root / (kind + '.json')).read_text())
            self.assertFalse(metadata['comparison_available'])
            self.assertFalse(metadata['pcm_latency_calibrated'])
            self.assertEqual(metadata['encoder']['settings_readback']['dtx'], 0)


if __name__ == '__main__':
    unittest.main()
