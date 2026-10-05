import copy
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import unittest

SCRIPT = Path(__file__).parents[1] / 'pn-opus-fixture.py'
spec = importlib.util.spec_from_file_location('pn_opus_fixture', SCRIPT)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
RUN = '2d616d44-906e-434d-94b8-553d8d903ee6'
NODE = Path(os.environ.get('GELABBER_PN_NODE', 'node'))


class PnOpusFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.opus = fixture.Opus(os.environ.get('GELABBER_PN_OPUS_LIBRARY'))
        cls.files, cls.loaded = {}, {}
        for kind in ('mic', 'source'):
            data, _samples = fixture.build(kind, 20, RUN, cls.opus, NODE)
            cls.files[kind] = data
            cls.loaded[kind] = fixture.parse(data, cls.opus, kind, NODE)

    def mutate(self, change):
        metadata, packets, _ = self.loaded['mic']
        metadata, packets = copy.deepcopy(metadata), list(packets)
        change(metadata, packets)
        return fixture.serialize(metadata, packets)

    def test_actual_two_sources_share_codebook_but_not_content_and_decode_every_packet(self):
        mic, source = [self.loaded[kind][0] for kind in ('mic', 'source')]
        self.assertEqual(mic['run_id'], source['run_id'])
        self.assertEqual(mic['pn']['codebook_sha256'], source['pn']['codebook_sha256'])
        self.assertNotEqual(mic['encoded_packets_sha256'], source['encoded_packets_sha256'])
        for kind, (metadata, packets, decoded) in self.loaded.items():
            self.assertEqual(len(packets), 1050)
            self.assertEqual(len(decoded), 1050 * 960)
            self.assertEqual(metadata['pn']['source_uid'], 0 if kind == 'mic' else 64)
            self.assertEqual(metadata['marker_control']['markers'], 10)
            self.assertLessEqual(metadata['marker_control']['max_alignment_error_samples'], 96)
            self.assertFalse(metadata['pcm_latency_calibrated'])
            self.assertEqual(metadata['provenance']['library_version'], 'libopus 1.6.1')

    def test_whole_360_seconds_codebook_is_unique_and_packet_aligned_with_explicit_tail(self):
        mic = fixture.codebook('mic', 360, RUN)
        source = fixture.codebook('source', 360, RUN)
        self.assertEqual(mic['codebook_sha256'], source['codebook_sha256'])
        self.assertEqual(len(mic['markers']), 180)
        all_codes = [tuple(row['code']) for book in (mic, source) for row in book['markers']]
        self.assertEqual(len(set(all_codes)), 360)
        for marker in mic['markers']:
            self.assertEqual(marker['source_sample_ordinal'] % 960, 0)
            self.assertLess(marker['source_sample_ordinal'] + 6048, 360 * 48000)
        self.assertEqual(mic['shared_run']['tail_samples'], 48000)
        self.assertNotEqual(mic['codebook_sha256'], fixture.codebook('mic', 360, '0bdd2f3c-2796-4c05-a43f-3b372fbd5369')['codebook_sha256'])

    def test_wrong_uid_or_role_is_rejected_even_when_archive_is_otherwise_valid(self):
        with self.assertRaises(ValueError): fixture.parse(self.files['mic'], self.opus, 'source', NODE)
        with self.assertRaises(subprocess.CalledProcessError):
            fixture.marker_control(self.loaded['mic'][2], fixture.codebook('source', 20, RUN), self.opus.configured_lookahead(), NODE)

    def test_forged_marker_code_start_uid_and_shared_run_are_rejected(self):
        mutations = [lambda m: m['pn']['markers'][0].update(source_sample_ordinal=0),
                     lambda m: m['pn']['markers'][0]['code'].__setitem__(0, -m['pn']['markers'][0]['code'][0]),
                     lambda m: m['pn'].update(source_uid=64),
                     lambda m: m['pn']['shared_run'].update(run_id='0bdd2f3c-2796-4c05-a43f-3b372fbd5369'),
                     lambda m: m['pn'].update(codebook_sha256='0' * 64)]
        for change in mutations:
            with self.subTest(change=change), self.assertRaises(ValueError):
                fixture.parse(self.mutate(lambda metadata, _packets: change(metadata)), self.opus, 'mic', NODE)

    def test_real_duplicate_after_300ms_is_ambiguous_in_actual_decoded_pcm(self):
        samples, _pcm, book = fixture.waveform('mic', 20, RUN, duplicate=True)
        packets, encoder = self.opus.encode(samples)
        decoded = self.opus.decode(packets)
        with self.assertRaises(subprocess.CalledProcessError) as error:
            fixture.marker_control(decoded, book, encoder['lookahead_samples'], NODE)
        self.assertIn('ambiguous', error.exception.stderr)

    def test_real_markerless_encoded_pcm_cannot_support_a_claimed_codebook(self):
        samples, _pcm, book = fixture.waveform('source', 20, RUN, markers=False)
        packets, encoder = self.opus.encode(samples)
        with self.assertRaises(subprocess.CalledProcessError):
            fixture.marker_control(self.opus.decode(packets), book, encoder['lookahead_samples'], NODE)

    def test_actual_clipping_is_rejected_in_source_and_decoded_samples(self):
        with self.assertRaises(ValueError): fixture.waveform('source', 20, RUN, gain=1)
        decoded = self.loaded['source'][2][:]
        decoded[123] = 1.2
        with self.assertRaises(subprocess.CalledProcessError) as error:
            fixture.marker_control(decoded, self.loaded['source'][0]['pn'], self.opus.configured_lookahead(), NODE)
        self.assertIn('clipped', error.exception.stderr)

    def test_byte_duration_channel_and_sample_ordinal_integrity(self):
        data = self.files['mic']
        for bad in (data[:8], data[:-1], data + data[-332:]):
            with self.subTest(length=len(bad)), self.assertRaises(ValueError): fixture.parse(bad, self.opus, 'mic', NODE)
        bad = bytearray(data); length, = struct.unpack_from('!I', data, 8)
        struct.pack_into('!Q', bad, 12 + length, 1)
        with self.assertRaises(ValueError): fixture.parse(bytes(bad), self.opus, 'mic', NODE)
        def stereo(metadata, packets):
            packets[:] = [bytes([packet[0] | 4]) + packet[1:] for packet in packets]
            metadata['encoded_packets_sha256'] = fixture.fixed.digest(b''.join(packets))
        with self.assertRaises(ValueError): fixture.parse(self.mutate(stereo), self.opus, 'mic', NODE)

    def test_false_calibration_duration_and_decoder_provenance_claims_are_rejected(self):
        changes = [lambda m: m.update(pcm_latency_calibrated=True),
                   lambda m: m.update(pcm_latency_calibrated=0),
                   lambda m: m.update(comparison_available=True),
                   lambda m: m.update(duration_seconds=20),
                   lambda m: m.update(tail_samples=0),
                   lambda m: m['provenance'].update(library_sha256='0' * 64),
                   lambda m: m['provenance'].update(pcm_kernel_sha256='0' * 64),
                   lambda m: m['encoder'].update(lookahead_samples=0)]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                fixture.parse(self.mutate(lambda metadata, _packets: change(metadata)), self.opus, 'mic', NODE)

    def test_duplicate_json_keys_cannot_forge_policy(self):
        data = self.files['mic']; length, = struct.unpack_from('!I', data, 8)
        header = data[12:12 + length]
        forged = b'{"schema":2,' + header[1:]
        bad = fixture.MAGIC + struct.pack('!I', len(forged)) + forged + data[12 + length:]
        with self.assertRaises(ValueError): fixture.parse(bad, self.opus, 'mic', NODE)


if __name__ == '__main__': unittest.main()
