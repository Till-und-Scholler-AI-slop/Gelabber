#!/usr/bin/env python3
"""Offline, fixed Opus packets for native peer0 and its separate source audio.

No server, socket or real-time encoder is started. Replay the same archive on
every engine; this instrument does not calibrate any end-to-end sample clock.
"""
import argparse
from array import array
import ctypes as c
import ctypes.util
import hashlib
import json
import math
from pathlib import Path
import platform
import struct
import sys

MAGIC = b"GPOPUS1\n"
RATE, CHANNELS, SAMPLES, PACKETS, BITRATE = 48_000, 1, 960, 500, 128_000
PERIOD_NS = 20_000_000
# Public libopus CTLs, verified against opus_defines.h; query every setting back.
CONTROLS = {"bitrate": (4002, 128_000), "vbr": (4006, 0),
            "complexity": (4010, 10), "inband_fec": (4012, 1),
            "packet_loss_percent": (4014, 1), "dtx": (4016, 0)}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pcm_bytes(samples):
    copy = array('f', samples)
    if sys.byteorder != 'little':
        copy.byteswap()
    return copy.tobytes()


def pcm(kind):
    if kind not in ('mic', 'source'):
        raise ValueError('unknown waveform')
    tones, gain = ([317, 719, 1249, 2027], .07) if kind == 'mic' else ([440], 1.)
    samples = array('f', (sum(gain * math.sin(2 * math.pi * frequency * i / RATE)
                              for frequency in tones) for i in range(SAMPLES * PACKETS)))
    return samples, {"kind": kind, "frequencies_hz": tones, "gain_per_tone": gain,
                     "phase_at_start_radians": 0, "samples": len(samples),
                     "format": "float32le mono", "sha256": digest(pcm_bytes(samples)),
                     "peak": max(abs(value) for value in samples)}


class Opus:
    def __init__(self, library=None):
        selected = str(library) if library else ctypes.util.find_library('opus')
        if not selected:
            raise ValueError('libopus is required for this offline instrument')
        self.lib = c.CDLL(selected)
        self.lib.opus_get_version_string.restype = c.c_char_p
        for name in ('opus_encoder_create', 'opus_decoder_create'):
            fn = getattr(self.lib, name); fn.restype = c.c_void_p
            fn.argtypes = [c.c_int, c.c_int] + ([c.c_int] if 'encoder' in name else []) + [c.POINTER(c.c_int)]
        for name in ('opus_encoder_destroy', 'opus_decoder_destroy'):
            getattr(self.lib, name).argtypes = [c.c_void_p]
        # opus_encoder_ctl is variadic. Type the fixed prefix and explicitly
        # pass each extra int/pointer; never allow pointer truncation.
        self.lib.opus_encoder_ctl.argtypes = [c.c_void_p, c.c_int]
        self.lib.opus_encoder_ctl.restype = c.c_int
        self.lib.opus_encode_float.argtypes = [c.c_void_p, c.POINTER(c.c_float), c.c_int, c.POINTER(c.c_ubyte), c.c_int]
        self.lib.opus_encode_float.restype = c.c_int
        self.lib.opus_decode_float.argtypes = [c.c_void_p, c.POINTER(c.c_ubyte), c.c_int, c.POINTER(c.c_float), c.c_int, c.c_int]
        self.lib.opus_decode_float.restype = c.c_int
        self.lib.opus_packet_get_nb_samples.argtypes = [c.POINTER(c.c_ubyte), c.c_int, c.c_int]
        self.lib.opus_packet_get_nb_samples.restype = c.c_int
        self.lib.opus_packet_get_nb_channels.argtypes = [c.POINTER(c.c_ubyte)]
        self.lib.opus_packet_get_nb_channels.restype = c.c_int
        mapped = [Path(line.split()[-1]).resolve() for line in Path('/proc/self/maps').read_text().splitlines()
                  if 'libopus.so' in line and line.split()[-1].startswith('/')]
        paths = set(mapped)
        if len(paths) != 1:
            raise ValueError('cannot identify exactly one executed libopus mapping')
        self.path = paths.pop()

    def duration(self, packet):
        raw = (c.c_ubyte * len(packet)).from_buffer_copy(packet)
        return self.lib.opus_packet_get_nb_samples(raw, len(packet), RATE)

    def channels(self, packet):
        raw = (c.c_ubyte * len(packet)).from_buffer_copy(packet)
        return self.lib.opus_packet_get_nb_channels(raw)

    def configured_lookahead(self):
        error = c.c_int(); encoder = self.lib.opus_encoder_create(RATE, CHANNELS, 2049, c.byref(error))
        if error.value or not encoder:
            raise ValueError('cannot query configured encoder lookahead')
        try:
            for request, value in CONTROLS.values():
                if self.lib.opus_encoder_ctl(encoder, request, c.c_int(value)):
                    raise ValueError('cannot restore configured encoder policy')
            actual = c.c_int()
            if self.lib.opus_encoder_ctl(encoder, 4027, c.byref(actual)):
                raise ValueError('cannot query actual configured lookahead')
            return actual.value
        finally:
            self.lib.opus_encoder_destroy(encoder)

    def encode(self, samples):
        if len(samples) != SAMPLES * PACKETS or array('f').itemsize != 4:
            raise ValueError('requires exactly 480000 float32 PCM samples')
        error = c.c_int(); encoder = self.lib.opus_encoder_create(RATE, CHANNELS, 2049, c.byref(error))
        if error.value or not encoder:
            raise ValueError('opus encoder creation failed')
        actual = {}
        try:
            for name, (request, value) in CONTROLS.items():
                if self.lib.opus_encoder_ctl(encoder, request, c.c_int(value)):
                    raise ValueError('opus setting rejected: ' + name)
                read = c.c_int()
                if self.lib.opus_encoder_ctl(encoder, request + 1, c.byref(read)) or read.value != value:
                    raise ValueError('opus setting readback differs: ' + name)
                actual[name] = read.value
            lookahead = c.c_int()
            if self.lib.opus_encoder_ctl(encoder, 4027, c.byref(lookahead)) or lookahead.value < 0:
                raise ValueError('cannot query actual codec lookahead')
            raw = (c.c_float * len(samples)).from_buffer(samples)
            output = (c.c_ubyte * 4000)(); packets = []
            for index in range(PACKETS):
                pointer = c.cast(c.byref(raw, index * SAMPLES * c.sizeof(c.c_float)), c.POINTER(c.c_float))
                length = self.lib.opus_encode_float(encoder, pointer, SAMPLES, output, len(output))
                if length != BITRATE * PERIOD_NS // (8 * 1_000_000_000):
                    raise ValueError('actual CBR packet is not exactly 320 bytes')
                packet = bytes(output[:length])
                if self.duration(packet) != SAMPLES or self.channels(packet) != CHANNELS:
                    raise ValueError('Opus packet duration/channel count differs')
                packets.append(packet)
            return packets, {"application": "audio", "application_constant": 2049,
                             "settings_readback": actual, "lookahead_samples": lookahead.value,
                             "lookahead_ms": lookahead.value / RATE * 1000,
                             "fec_note": "configured FEC is not proof of a particular packet carrying FEC"}
        finally:
            self.lib.opus_encoder_destroy(encoder)

    def decode(self, packets):
        error = c.c_int(); decoder = self.lib.opus_decoder_create(RATE, CHANNELS, c.byref(error))
        if error.value or not decoder:
            raise ValueError('opus decoder creation failed')
        samples = array('f'); output = (c.c_float * SAMPLES)()
        try:
            for packet in packets:
                raw = (c.c_ubyte * len(packet)).from_buffer_copy(packet)
                length = self.lib.opus_decode_float(decoder, raw, len(packet), output, SAMPLES, 0)
                if length != SAMPLES:
                    raise ValueError('every packet must decode to exactly 960 samples')
                samples.extend(output)
        finally:
            self.lib.opus_decoder_destroy(decoder)
        if len(samples) != PACKETS * SAMPLES or not all(math.isfinite(value) for value in samples):
            raise ValueError('missing/nonfinite decoded PCM')
        return samples

    def provenance(self):
        return {"api": "direct opus_encode_float/opus_decode_float; no Ogg trim/flush",
                "library_path": str(self.path), "library_sha256": digest(self.path.read_bytes()),
                "library_version": self.lib.opus_get_version_string().decode(),
                "python_version": platform.python_version(), "machine": platform.machine(),
                "python_binary_sha256": digest(Path(sys.executable).resolve().read_bytes()),
                "script_sha256": digest(Path(__file__).read_bytes())}


def serialize(metadata, packets):
    header = json.dumps(metadata, sort_keys=True, separators=(',', ':')).encode()
    return MAGIC + struct.pack('!I', len(header)) + header + b''.join(
        struct.pack('!QI', index * PERIOD_NS, len(packet)) + packet
        for index, packet in enumerate(packets))


def parse(data, opus):
    if not 12 <= len(data) <= 256 * 1024 or data[:8] != MAGIC:
        raise ValueError('invalid/truncated Opus archive magic')
    length, = struct.unpack_from('!I', data, 8)
    if not 1 <= length <= 64 * 1024 or 12 + length > len(data):
        raise ValueError('invalid/truncated Opus archive metadata')
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate Opus metadata key')
            result[key] = value
        return result
    metadata = json.loads(data[12:12 + length], object_pairs_hook=unique); position = 12 + length; packets = []
    expected = {"schema": 1, "codec": "opus", "sample_rate_hz": RATE, "channels": CHANNELS,
                "packet_duration_ms": 20, "packets": PACKETS, "duration_seconds": 10,
                "rtp_clock_hz": RATE, "rtp_timestamp_step": SAMPLES, "payload_bitrate_bps": BITRATE,
                "encoded_bytes": 160_000}
    if not isinstance(metadata, dict) or any(metadata.get(key) != value for key, value in expected.items()):
        raise ValueError('Opus source policy differs')
    if metadata.get('encoder', {}).get('settings_readback') != {name: value for name, (_, value) in CONTROLS.items()}:
        raise ValueError('Opus encoder policy differs')
    while position < len(data):
        if len(packets) >= PACKETS or position + 12 > len(data):
            raise ValueError('extra/truncated Opus packet record')
        due, size = struct.unpack_from('!QI', data, position); position += 12
        if due != len(packets) * PERIOD_NS or size != 320 or position + size > len(data):
            raise ValueError('wrong schedule/length or truncated Opus packet')
        packet = data[position:position + size]; position += size
        if opus.duration(packet) != SAMPLES or opus.channels(packet) != CHANNELS:
            raise ValueError('packet declares wrong Opus duration/channel count')
        packets.append(packet)
    if len(packets) != PACKETS or digest(b''.join(packets)) != metadata.get('encoded_packets_sha256'):
        raise ValueError('missing or altered encoded Opus packets')
    decoded = opus.decode(packets)
    # Producer labels are part of the source contract, not measured facts.
    # Verify the named waveform and actual decoded content before returning it.
    claimed_pcm = metadata.get('pcm')
    if not isinstance(claimed_pcm, dict):
        raise ValueError('missing PCM source contract')
    original, actual_pcm = pcm(claimed_pcm.get('kind'))
    if claimed_pcm != actual_pcm:
        raise ValueError('declared PCM differs from the fixed source waveform')
    lookahead = metadata.get('encoder', {}).get('lookahead_samples')
    if not isinstance(lookahead, int) or lookahead != opus.configured_lookahead():
        raise ValueError('declared lookahead differs from actual configured codec')
    if metadata['encoder'].get('application') != 'audio' or metadata['encoder'].get('application_constant') != 2049:
        raise ValueError('encoder application differs from the source policy')
    actual = decode_control(decoded, original, lookahead)
    claimed = metadata.get('decode_control')
    if not isinstance(claimed, dict) or actual != claimed or actual['input_correlation_after_codec_lookahead'] < .98:
        raise ValueError('declared decoder control differs from actual decoded PCM')
    return metadata, packets, decoded


def decode_control(decoded, original, lookahead):
    aligned = decoded[lookahead:]; reference = original[:len(aligned)]
    energy = sum(float(value) ** 2 for value in aligned)
    input_energy = sum(float(value) ** 2 for value in reference)
    correlation = sum(a * b for a, b in zip(aligned, reference)) / math.sqrt(energy * input_energy) if energy and input_energy else 0
    return {"samples": len(decoded), "float32le_sha256": digest(pcm_bytes(decoded)),
            "peak": max(abs(value) for value in decoded), "input_correlation_after_codec_lookahead": correlation}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path)
    commands = parser.add_subparsers(dest='operation', required=True)
    build = commands.add_parser('build'); build.add_argument('kind', choices=['mic', 'source']); build.add_argument('archive', type=Path)
    build.add_argument('--output', type=Path, required=True); build.add_argument('--pcm', type=Path)
    inspect = commands.add_parser('inspect'); inspect.add_argument('archive', type=Path)
    args = parser.parse_args(); opus = Opus(args.library)
    if args.operation == 'inspect':
        metadata, packets, decoded = parse(args.archive.read_bytes(), opus)
        print(json.dumps({"archive_sha256": digest(args.archive.read_bytes()), "metadata": metadata,
                          "inspected_packets": len(packets), "decoded_samples": len(decoded),
                          "executed_decoder": opus.provenance()}, indent=2)); return
    targets = [args.archive, args.output] + ([args.pcm] if args.pcm else [])
    if len(set(path.resolve() for path in targets)) != len(targets) or any(path.exists() for path in targets):
        parser.error('all outputs must have distinct fresh paths')
    samples, waveform = pcm(args.kind); packets, settings = opus.encode(samples); decoded = opus.decode(packets)
    lookahead = settings['lookahead_samples']
    actual_decode = decode_control(decoded, samples, lookahead)
    correlation = actual_decode['input_correlation_after_codec_lookahead']
    if not math.isfinite(correlation) or correlation < .98:
        raise ValueError('decoded waveform does not correlate with the actual generated input')
    metadata = {"schema": 1, "codec": "opus", "sample_rate_hz": RATE, "channels": CHANNELS,
                "packet_duration_ms": 20, "packets": PACKETS, "duration_seconds": 10,
                "rtp_clock_hz": RATE, "rtp_timestamp_step": SAMPLES, "payload_bitrate_bps": BITRATE,
                "encoded_bytes": sum(map(len, packets)), "encoded_packets_sha256": digest(b''.join(packets)),
                "pcm": waveform, "encoder": settings, "provenance": opus.provenance(),
                "decode_control": actual_decode,
                "schedule": "500 packets, one per 20ms; native replay must advance timestamps across loops",
                "loop_policy": "reuse frozen bytes; initial codec lookahead is retained, no sample-exact loop or latency claim",
                "comparison_available": False, "pcm_latency_calibrated": False}
    archive = serialize(metadata, packets); parse(archive, opus)
    with args.archive.open('xb') as output: output.write(archive)
    if args.pcm:
        with args.pcm.open('xb') as output: output.write(pcm_bytes(samples))
    with args.output.open('x') as output:
        json.dump({**metadata, "archive_sha256": digest(archive)}, output, indent=2); output.write('\n')
    print(json.dumps({"archive": str(args.archive), "archive_sha256": digest(archive),
                      "packets": len(packets), "payload_bitrate_bps": BITRATE, "correlation": correlation}))


if __name__ == '__main__':
    main()
