#!/usr/bin/env python3
"""Unlooped UID0/64 PN Opus archives. Offline integrity, never PCM latency qualification."""
import argparse
from array import array
import ctypes as c
import importlib.util
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import uuid

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('fixed_opus', HERE / 'opus-fixture.py')
fixed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixed)
MAGIC = b'GPOPUS2\n'
RATE, FRAME, PERIOD_NS = 48000, 960, 20_000_000
MARKER = {'sampleRate': RATE, 'carrierHz': 2000, 'chipFrames': 96, 'chips': 63,
          'periodFrames': 96000, 'amplitude': .35, 'threshold': .72}
LIBRARY_VERSION = 'libopus 1.6.1'


def code(uid, sequence):
    seed = (((uid + 1) * 0x9e3779b1) ^ (sequence * 0x85ebca6b)) & 0xffffffff
    result = []
    for _ in range(MARKER['chips']):
        seed ^= (seed << 13) & 0xffffffff
        seed ^= seed >> 17
        seed ^= (seed << 5) & 0xffffffff
        seed &= 0xffffffff
        result.append(1 if seed & 1 else -1)
    return result


def codebook(kind, seconds, run_id):
    if kind not in ('mic', 'source') or type(seconds) is not int or not 20 <= seconds <= 360:
        raise ValueError('requires mic/source and whole measurement duration20..360 seconds')
    if not isinstance(run_id, str) or str(uuid.UUID(run_id)) != run_id:
        raise ValueError('requires canonical shared run UUID')
    uid = 0 if kind == 'mic' else 64
    markers = [{'sequence': sequence, 'source_sample_ordinal': start, 'code': code(uid, sequence)}
               for sequence, start in enumerate(range(RATE, seconds * RATE - 6048, 96000))]
    # Quadrature correlation also treats a globally inverted code as a match.
    signatures = set()
    for source in (0, 64):
        for marker in markers:
            signs = code(source, marker['sequence'])
            signature = min(tuple(signs), tuple(-value for value in signs))
            if signature in signatures: raise ValueError('duplicate/inverted PN code within run')
            signatures.add(signature)
    sources = [{'kind': role, 'source_uid': source, 'ssrc': ssrc,
                'markers': [{**marker, 'code': code(source, marker['sequence'])} for marker in markers]}
               for role, source, ssrc in [('mic', 0, 0x474d4943), ('source', 64, 0x47534130)]]
    shared = {'run_id': run_id, 'measurement_end_sample_ordinal': seconds * RATE,
              'tail_samples': RATE, 'marker_policy': MARKER, 'sources': sources}
    shared_hash = fixed.digest(json.dumps(shared, sort_keys=True, separators=(',', ':')).encode())
    return {'run_id': run_id, 'codebook_sha256': shared_hash, 'source_uid': uid,
            'ssrc': sources[0 if kind == 'mic' else 1]['ssrc'], 'marker_policy': MARKER,
            'markers': markers, 'shared_run': shared}


def waveform(kind, seconds, run_id, *, markers=True, gain=None, duplicate=False):
    book = codebook(kind, seconds, run_id)
    frequencies, default_gain = ([317, 719, 1249, 2027], .07) if kind == 'mic' else ([440], .45)
    tone_gain = default_gain if gain is None else gain
    samples = array('f', (sum(tone_gain * math.sin(2 * math.pi * frequency * frame / RATE)
                              for frequency in frequencies) for frame in range((seconds + 1) * RATE)))
    if markers:
        for marker in book['markers']:
            for delay in ([0, 14400] if duplicate else [0]):
                start = marker['source_sample_ordinal'] + delay
                for offset in range(6048):
                    samples[start + offset] += marker['code'][offset // 96] * math.sin(offset * 2 * math.pi * 2000 / RATE) * .35
    peak = max(abs(value) for value in samples)
    if not all(math.isfinite(value) for value in samples) or peak >= .999:
        raise ValueError('nonfinite or clipped source PCM')
    pcm = {'kind': kind, 'source_uid': book['source_uid'], 'frequencies_hz': frequencies,
           'gain_per_tone': tone_gain, 'samples': len(samples), 'format': 'float32le mono',
           'sha256': fixed.digest(fixed.pcm_bytes(samples)), 'peak': peak}
    return samples, pcm, book


class Opus(fixed.Opus):
    def __init__(self, library=None):
        super().__init__(library)
        if self.lib.opus_get_version_string().decode() != LIBRARY_VERSION:
            raise ValueError('actual mapped libopus must be1.6.1')

    def encode(self, samples):
        if not samples or len(samples) % FRAME or array('f').itemsize != 4:
            raise ValueError('requires whole 960-sample float32 frames')
        error = c.c_int()
        encoder = self.lib.opus_encoder_create(RATE, 1, 2049, c.byref(error))
        if error.value or not encoder: raise ValueError('encoder creation failed')
        actual = {}
        try:
            for name, (request, value) in fixed.CONTROLS.items():
                read = c.c_int()
                if self.lib.opus_encoder_ctl(encoder, request, c.c_int(value)) or self.lib.opus_encoder_ctl(encoder, request + 1, c.byref(read)) or read.value != value:
                    raise ValueError('encoder control readback differs: ' + name)
                actual[name] = read.value
            lookahead = c.c_int()
            if self.lib.opus_encoder_ctl(encoder, 4027, c.byref(lookahead)) or lookahead.value < 0:
                raise ValueError('lookahead query failed')
            raw = (c.c_float * len(samples)).from_buffer(samples)
            output = (c.c_ubyte * 4000)()
            packets = []
            for index in range(len(samples) // FRAME):
                pointer = c.cast(c.byref(raw, index * FRAME * c.sizeof(c.c_float)), c.POINTER(c.c_float))
                length = self.lib.opus_encode_float(encoder, pointer, FRAME, output, len(output))
                packet = bytes(output[:max(0, length)])
                if length != 320 or self.duration(packet) != FRAME or self.channels(packet) != 1:
                    raise ValueError('actual Opus length/duration/channels differs')
                packets.append(packet)
            return packets, {'application': 'audio', 'application_constant': 2049, 'settings_readback': actual,
                             'lookahead_samples': lookahead.value, 'lookahead_ms': lookahead.value / RATE * 1000,
                             'fec_note': 'FEC configured, actual FEC per packet not asserted'}
        finally:
            self.lib.opus_encoder_destroy(encoder)

    def decode(self, packets):
        error = c.c_int()
        decoder = self.lib.opus_decoder_create(RATE, 1, c.byref(error))
        if error.value or not decoder: raise ValueError('decoder creation failed')
        samples, output = array('f'), (c.c_float * FRAME)()
        try:
            for packet in packets:
                if len(packet) != 320 or self.duration(packet) != FRAME or self.channels(packet) != 1:
                    raise ValueError('actual Opus packet policy differs')
                raw = (c.c_ubyte * len(packet)).from_buffer_copy(packet)
                if self.lib.opus_decode_float(decoder, raw, len(packet), output, FRAME, 0) != FRAME:
                    raise ValueError('packet did not decode to960 samples')
                samples.extend(output)
        finally:
            self.lib.opus_decoder_destroy(decoder)
        if not all(math.isfinite(value) for value in samples): raise ValueError('nonfinite decoded PCM')
        return samples

    def provenance(self):
        return {**super().provenance(), 'script_sha256': fixed.digest(Path(__file__).read_bytes()),
                'fixed_opus_script_sha256': fixed.digest((HERE / 'opus-fixture.py').read_bytes()),
                'pcm_kernel_sha256': fixed.digest((HERE / 'pcm-kernel.mjs').read_bytes()),
                'pn_inspector_sha256': fixed.digest((HERE / 'pn-opus-markers.mjs').read_bytes())}


def marker_control(decoded, book, lookahead, node):
    node_path = Path(node).resolve(strict=True)
    version = subprocess.check_output([str(node_path), '--version'], text=True).strip()
    if version != 'v26.8.2': raise ValueError('actual Node must be26.8.2')
    with tempfile.TemporaryDirectory(prefix='gelabber-pn-decode-') as folder:
        root = Path(folder)
        (root / 'decoded.f32').write_bytes(fixed.pcm_bytes(decoded))
        (root / 'book.json').write_text(json.dumps(book))
        result = subprocess.run([str(node), str(HERE / 'pn-opus-markers.mjs'), str(root / 'decoded.f32'), str(root / 'book.json'), str(lookahead)], capture_output=True, text=True, check=True, timeout=120)
        return {**json.loads(result.stdout), 'executed_node': {'version': version, 'sha256': fixed.digest(node_path.read_bytes())}}


def serialize(metadata, packets):
    header = json.dumps(metadata, sort_keys=True, separators=(',', ':')).encode()
    return MAGIC + struct.pack('!I', len(header)) + header + b''.join(struct.pack('!QI', index * PERIOD_NS, len(packet)) + packet for index, packet in enumerate(packets))


def parse(data, opus, expected_kind, node):
    if not 12 <= len(data) <= 8 * 1024 * 1024 or data[:8] != MAGIC:
        raise ValueError('invalid PN archive magic/length')
    length, = struct.unpack_from('!I', data, 8)
    if not 1 <= length <= 256 * 1024 or 12 + length > len(data): raise ValueError('invalid manifest length')
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result: raise ValueError('duplicate manifest key')
            result[key] = value
        return result
    metadata = json.loads(data[12:12 + length], object_pairs_hook=unique)
    if not isinstance(metadata, dict): raise ValueError('invalid manifest')
    seconds = metadata.get('measurement_seconds')
    book = codebook(expected_kind, seconds, metadata.get('run_id'))
    count = (seconds + 1) * 50
    expected = {'schema': 2, 'codec': 'opus', 'sample_rate_hz': RATE, 'channels': 1,
                'packet_duration_ms': 20, 'packets': count, 'measurement_seconds': seconds,
                'tail_seconds': 1, 'duration_seconds': seconds + 1, 'rtp_clock_hz': RATE,
                'measurement_end_sample_ordinal': seconds * RATE, 'tail_samples': RATE,
                'rtp_timestamp_step': FRAME, 'payload_bitrate_bps': 128000, 'encoded_bytes': count * 320,
                'loop_policy': 'unlooped; stop at archive end; no rewind or modulo',
                'pcm_latency_calibrated': False, 'comparison_available': False}
    if any(type(metadata.get(key)) is not type(value) or metadata.get(key) != value for key, value in expected.items()) or json.dumps(metadata.get('pn'), sort_keys=True) != json.dumps(book, sort_keys=True):
        raise ValueError('PN source identity/codebook/policy differs')
    packets, cursor = [], 12 + length
    while cursor < len(data):
        if len(packets) >= count or cursor + 12 > len(data): raise ValueError('extra/truncated packet')
        due, size = struct.unpack_from('!QI', data, cursor); cursor += 12
        if due != len(packets) * PERIOD_NS or size != 320 or cursor + size > len(data): raise ValueError('wrong packet schedule/length')
        packets.append(data[cursor:cursor + size]); cursor += size
    if len(packets) != count or metadata.get('encoded_packets_sha256') != fixed.digest(b''.join(packets)):
        raise ValueError('missing/altered packets')
    actual_provenance = opus.provenance()
    if any(metadata.get('provenance', {}).get(key) != actual_provenance[key] for key in ('library_version', 'library_sha256', 'script_sha256', 'fixed_opus_script_sha256', 'pcm_kernel_sha256', 'pn_inspector_sha256')):
        raise ValueError('executed library/script/kernel provenance differs')
    encoder = metadata.get('encoder', {})
    lookahead = opus.configured_lookahead()
    if encoder.get('settings_readback') != {name: value for name, (_, value) in fixed.CONTROLS.items()} or encoder.get('lookahead_samples') != lookahead or encoder.get('lookahead_ms') != lookahead / RATE * 1000 or encoder.get('application') != 'audio' or encoder.get('application_constant') != 2049:
        raise ValueError('actual encoder policy/lookahead differs')
    original, pcm, _ = waveform(expected_kind, seconds, metadata['run_id'])
    if metadata.get('pcm') != pcm: raise ValueError('forged source PCM claim')
    decoded = opus.decode(packets)
    control = fixed.decode_control(decoded, original, lookahead)
    if metadata.get('decode_control') != control or control['input_correlation_after_codec_lookahead'] < .98 or control['peak'] >= .999:
        raise ValueError('forged/uncorrelated/clipped decoded PCM')
    markers = marker_control(decoded, book, lookahead, node)
    if metadata.get('marker_control') != markers: raise ValueError('forged decoded marker claim')
    return metadata, packets, decoded


def build(kind, seconds, run_id, opus, node):
    samples, pcm, book = waveform(kind, seconds, run_id)
    packets, encoder = opus.encode(samples)
    decoded = opus.decode(packets)
    control = fixed.decode_control(decoded, samples, encoder['lookahead_samples'])
    if control['input_correlation_after_codec_lookahead'] < .98 or control['peak'] >= .999:
        raise ValueError('decoded content correlation/clipping gate failed')
    metadata = {'schema': 2, 'codec': 'opus', 'sample_rate_hz': RATE, 'channels': 1,
                'packet_duration_ms': 20, 'packets': len(packets), 'measurement_seconds': seconds,
                'run_id': run_id, 'measurement_end_sample_ordinal': seconds * RATE, 'tail_samples': RATE,
                'tail_seconds': 1, 'duration_seconds': seconds + 1, 'rtp_clock_hz': RATE,
                'rtp_timestamp_step': FRAME, 'payload_bitrate_bps': 128000, 'encoded_bytes': sum(map(len, packets)),
                'encoded_packets_sha256': fixed.digest(b''.join(packets)), 'pcm': pcm, 'pn': book,
                'encoder': encoder, 'provenance': opus.provenance(), 'decode_control': control,
                'marker_control': marker_control(decoded, book, encoder['lookahead_samples'], node),
                'loop_policy': 'unlooped; stop at archive end; no rewind or modulo',
                'schedule_scope': 'packet enqueued at planned FIRST stored PCM sample; offline encoder/capture/DSP excluded',
                'pcm_latency_calibrated': False, 'comparison_available': False}
    return serialize(metadata, packets), samples


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path)
    parser.add_argument('--node', type=Path, required=True)
    commands = parser.add_subparsers(dest='operation', required=True)
    make = commands.add_parser('build'); make.add_argument('kind', choices=('mic', 'source')); make.add_argument('archive', type=Path)
    make.add_argument('--seconds', type=int, default=120); make.add_argument('--output', type=Path, required=True); make.add_argument('--pcm', type=Path)
    make.add_argument('--run-id', required=True)
    inspect = commands.add_parser('inspect'); inspect.add_argument('kind', choices=('mic', 'source')); inspect.add_argument('archive', type=Path)
    args = parser.parse_args()
    opus = Opus(args.library)
    if args.operation == 'inspect':
        data = args.archive.read_bytes(); metadata, packets, decoded = parse(data, opus, args.kind, args.node)
        print(json.dumps({'archive_sha256': fixed.digest(data), 'packets': len(packets), 'decoded_samples': len(decoded), 'metadata': metadata, 'executed_decoder': opus.provenance()}, indent=2)); return
    paths = [args.archive, args.output] + ([args.pcm] if args.pcm else [])
    if len(set(path.resolve() for path in paths)) != len(paths) or any(path.exists() for path in paths): parser.error('all output paths must be distinct and fresh')
    data, samples = build(args.kind, args.seconds, args.run_id, opus, args.node)
    metadata, _, _ = parse(data, opus, args.kind, args.node)
    with args.archive.open('xb') as handle: handle.write(data)
    with args.output.open('x') as handle: json.dump({**metadata, 'archive_sha256': fixed.digest(data)}, handle, indent=2); handle.write('\n')
    if args.pcm:
        with args.pcm.open('xb') as handle: handle.write(fixed.pcm_bytes(samples))
    print(json.dumps({'archive': str(args.archive), 'sha256': fixed.digest(data), 'markers': len(metadata['pn']['markers']), 'packets': metadata['packets'], 'pcm_latency_calibrated': False}))


if __name__ == '__main__': main()
