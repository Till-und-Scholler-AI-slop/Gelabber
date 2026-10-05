#!/usr/bin/env python3
"""Inspect browser rate/queue counters and Chromium v2 RTC event logs.

Diagnostic only: batched protobuf timestamps are not delta-decoded. RTCP wire
packets are decoded completely, including all blob-batched packets. Unknown
protobuf fields/RTCP packet types are retained as counts, never inferred.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct


def varint(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        if offset >= len(data): raise ValueError('truncated varint')
        byte = data[offset]; offset += 1; value |= (byte & 127) << shift
        if byte < 128: return value, offset
    raise ValueError('oversized varint')


def fields(data):
    offset = 0
    while offset < len(data):
        tag, offset = varint(data, offset)
        number, wire = tag >> 3, tag & 7
        if number == 0: raise ValueError('invalid protobuf field')
        if wire == 0: value, offset = varint(data, offset)
        else:
            if wire == 2: length, offset = varint(data, offset)
            elif wire in (1, 5): length = 8 if wire == 1 else 4
            else: raise ValueError('unsupported protobuf wire type')
            if offset + length > len(data): raise ValueError('truncated protobuf field')
            value = data[offset:offset + length]; offset += length
        yield number, value


def blobs(data, count):
    if count > 100000: raise ValueError('oversized blob batch')
    offset, lengths = 0, []
    for _ in range(count):
        length, offset = varint(data, offset); lengths.append(length)
    for length in lengths:
        if offset + length > len(data): raise ValueError('truncated RTCP blob')
        yield data[offset:offset + length]; offset += length
    if offset != len(data): raise ValueError('unexpected RTCP blob trailer')


def twcc(packet):
    base, count = struct.unpack_from('!HH', packet, 12)
    offset, statuses = 20, []
    while len(statuses) < count:
        if offset + 2 > len(packet): raise ValueError('truncated TWCC chunk')
        chunk = int.from_bytes(packet[offset:offset + 2], 'big'); offset += 2
        if not chunk & 0x8000: statuses.extend([(chunk >> 13) & 3] * (chunk & 8191))
        elif not chunk & 0x4000: statuses.extend((chunk >> bit) & 1 for bit in range(13, -1, -1))
        else: statuses.extend((chunk >> bit) & 3 for bit in range(12, -1, -2))
        if not chunk: raise ValueError('empty TWCC run')
    statuses = statuses[:count]; deltas = []
    for status in statuses:
        if status == 0: continue
        if status == 1:
            if offset >= len(packet): raise ValueError('truncated TWCC small delta')
            value = packet[offset]; offset += 1
        elif status == 2:
            if offset + 2 > len(packet): raise ValueError('truncated TWCC large delta')
            value = struct.unpack_from('!h', packet, offset)[0]; offset += 2
        else: raise ValueError('reserved TWCC packet status')
        deltas.append(value * 250)
    return {'base_sequence': base, 'status_count': count, 'received_count': len(deltas),
            'reference_time_64ms': int.from_bytes(packet[16:19], 'big'), 'feedback_count': packet[19],
            'arrival_delta_us_min': min(deltas, default=None), 'arrival_delta_us_max': max(deltas, default=None),
            'arrival_delta_us_sum': sum(deltas)}


def rtcp(data):
    offset = 0
    while offset < len(data):
        if offset + 4 > len(data): raise ValueError('truncated RTCP header')
        first, kind, length = struct.unpack_from('!BBH', data, offset)
        size = (length + 1) * 4
        packet = data[offset:offset + size]; offset += size
        if first >> 6 != 2 or len(packet) != size: raise ValueError('invalid RTCP length/version')
        count = first & 31
        value = {'type': kind, 'format_or_count': count}
        if size >= 8: value['sender_ssrc'] = int.from_bytes(packet[4:8], 'big')
        if kind in (200, 201):
            start = 28 if kind == 200 else 8
            if start + count * 24 > size: raise ValueError('truncated RR report block')
            value['reports'] = [{'target_ssrc': int.from_bytes(packet[p:p + 4], 'big'),
                'fraction_lost': packet[p + 4], 'cumulative_lost': int.from_bytes(packet[p + 5:p + 8], 'big', signed=True),
                'highest_sequence': int.from_bytes(packet[p + 8:p + 12], 'big'),
                'jitter_rtp_units': int.from_bytes(packet[p + 12:p + 16], 'big')}
                for p in range(start, start + count * 24, 24)]
        if kind in (205, 206) and size >= 12: value['media_ssrc'] = int.from_bytes(packet[8:12], 'big')
        if kind == 205 and count == 15:
            if size < 20: raise ValueError('truncated TWCC feedback')
            value['twcc'] = twcc(packet)
        if kind == 206 and count == 15 and packet[12:16] == b'REMB':
            if size < 20 or 20 + packet[16] * 4 > size: raise ValueError('truncated REMB')
            mantissa = ((packet[17] & 3) << 16) | int.from_bytes(packet[18:20], 'big')
            value['remb'] = {'bitrate_bps': mantissa << (packet[17] >> 2),
                'target_ssrcs': [int.from_bytes(packet[p:p + 4], 'big') for p in range(20, 20 + packet[16] * 4, 4)]}
        yield value


def event_log(path):
    if path.stat().st_size > 32 * 1024 ** 2: raise ValueError('event log exceeds diagnostic size limit')
    data = path.read_bytes(); events, anchors, counts = [], [], Counter()
    for number, encoded in fields(data):
        counts[number] += 1
        if number in (18, 19):
            message = dict(fields(encoded))
            anchors.append({'type': 'loss' if number == 18 else 'delay', 'timestamp_ms': message.get(1),
                'bitrate_bps': message.get(2), 'fraction_loss_or_detector_state': message.get(3),
                'unparsed_delta_updates': message.get(5 if number == 18 else 4, 0)})
        if number not in (4, 5): continue
        message = dict(fields(encoded)); raw = [message[2]]
        if message.get(3): raw.extend(blobs(message.get(102, b''), message[3]))
        for index, packet in enumerate(raw):
            for value in rtcp(packet): events.append({'direction': 'incoming' if number == 4 else 'outgoing',
                'batch_anchor_timestamp_ms': message.get(1), 'batch_index': index, **value})
    return {'name': path.name, 'sha256': hashlib.sha256(data).hexdigest(), 'protobuf_event_counts': dict(counts),
            'rtcp': events, 'bwe_batch_first_updates': anchors, 'scope': 'RTCP targets/deltas; batch timestamps are anchors, not decoded event times'}


def browser_diagnostics(browser):
    first, last = browser['samples'][0], browser['samples'][-1]
    seconds = (last['at'] - first['at']) / 1000
    previous = {(s.get('_endpoint'), s['id']): s for s in first['stats']}
    streams = []
    for value in last['stats']:
        if value['type'] not in ('outbound-rtp', 'media-source') or value.get('kind') != 'video': continue
        before = previous.get((value.get('_endpoint'), value['id']))
        if not before: continue
        row = {'endpoint': value.get('_endpoint'), 'ssrc': value.get('ssrc'), 'type': value['type']}
        for field in ['frames', 'framesEncoded']:
            if field in value and field in before: row[field + '_per_second'] = (value[field] - before[field]) / seconds
        packets = value.get('packetsSent', 0) - before.get('packetsSent', 0)
        if packets > 0 and 'totalPacketSendDelay' in value and 'totalPacketSendDelay' in before:
            row['mean_packet_send_delay_ms'] = (value['totalPacketSendDelay'] - before['totalPacketSendDelay']) / packets * 1000
        row['packet_count_delta'] = packets
        row['target_bitrate_bps_last'] = value.get('targetBitrate')
        streams.append(row)
    return {'configuration': browser.get('protocol_configuration'), 'video': streams}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path, help='one case directory containing browser.json')
    args = parser.parse_args(); browser = json.loads((args.directory / 'browser.json').read_text())
    result = browser_diagnostics(browser)
    result['rtc_event_logs'], result['errors'] = [], []
    for path in sorted((args.directory / 'rtc-events').glob('*')):
        if not path.is_file(): continue
        try: result['rtc_event_logs'].append(event_log(path))
        except (ValueError, KeyError, struct.error) as error:
            result['errors'].append(path.name + ': ' + str(error))
            result['rtc_event_logs'].append({'name': path.name, 'error': str(error)})
    if browser.get('load_generator', {}).get('protocol_logs'):
        if not result['rtc_event_logs']: result['errors'].append('RTC event logging requested but no logs exist')
    (args.directory / 'protocol.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'video': result['video'], 'errors': result['errors']}, indent=2))
    return 1 if result['errors'] else 0


if __name__ == '__main__': raise SystemExit(main())
