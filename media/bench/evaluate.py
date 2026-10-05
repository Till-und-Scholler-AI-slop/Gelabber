#!/usr/bin/env python3
"""Summarize raw evidence without turning local probes into migration approval."""
import argparse
import json
from pathlib import Path
import statistics
import math


def media_stats(sample, direction):
    stats = sample.get('stats', [])
    codecs = {(s.get('_endpoint'), s['id']): s.get('mimeType', '').lower()
              for s in stats if s.get('type') == 'codec'}
    return [s for s in stats if s.get('type') == direction and s.get('mid') != 'probator'
            and not codecs.get((s.get('_endpoint'), s.get('codecId')), '').endswith('/rtx')]


def key(stream):
    return stream.get('_endpoint'), stream['id']


def differences(first, last, field, kind):
    previous = {key(s): s for s in first}
    return [s[field] - previous[key(s)][field] for s in last
            if s.get('kind') == kind and key(s) in previous
            and field in s and field in previous[key(s)]
            and s[field] >= previous[key(s)][field]]


def median(values):
    return statistics.median(values) if values else None


def same_comparison_browser(runs):
    if not runs or any(len({bool(run.get(policy)) for run in runs}) != 1 for policy in ['fixed_video_fixture', 'pcm_latency_enabled']):
        return False
    identities = {tuple(run.get('executed_browser', {}).get(field) for field in ['product', 'revision', 'sha256']) for run in runs}
    return len(identities) == 1 and all(next(iter(identities)))


def summarize_pcm(browser, problems):
    evidence = browser.get('pcm_latency', {})
    calibration = browser.get('pcm_calibration', {})
    parameters = evidence.get('parameters', {})
    if not calibration.get('valid') or calibration.get('failures') or len(calibration.get('delay_checks', [])) != 4 or not all(check.get('valid') for check in calibration.get('delay_checks', [])):
        problems.append('PCM measurement lacks passing known-delay calibration')
    actual = browser.get('load_generator', {}).get('executed_browser', {})
    if any(not actual.get(field) or actual[field] != calibration.get('executed_browser', {}).get(field) for field in ['product', 'revision', 'sha256']):
        problems.append('PCM calibration executed browser differs')
    if parameters != calibration.get('calibration', {}).get('parameters') or parameters.get('errorBoundMs') != 2 or parameters.get('sampleRate') != 48000:
        problems.append('PCM detector policy differs from calibration')
    problems.extend(evidence.get('failures', []))
    if evidence.get('clipped_frames', 0): problems.append('PCM source clipped')
    clock, wall = evidence.get('sample_clock_seconds', 0), evidence.get('wall_clock_seconds', 0)
    if min(clock, wall) < 8 or abs(clock - wall) > .1: problems.append('PCM sampleclock and wallclock differ or interval is insufficient')
    count = browser['peers']
    expected = {(f'peer-{receiver}', f'peer-{sender}/mic') for receiver in range(count) for sender in range(count) if sender != receiver}
    if browser['video']: expected.update((f'peer-{receiver}', 'peer-0/screen-audio') for receiver in range(1, count))
    edges = evidence.get('edges', [])
    observed = {(edge.get('peer'), edge.get('source')) for edge in edges}
    if observed != expected or len(edges) != len(expected): problems.append('PCM graph lacks unique source identity on every audio edge')
    summaries, all_values = [], []
    for edge in edges:
        matches = edge.get('matches', [])
        values = [match.get('latency_ms') for match in matches if not match.get('problem')]
        valid = edge.get('expected_markers', 0) >= 3 and len(matches) == edge.get('expected_markers') and len(values) == len(matches)
        valid = valid and all(isinstance(value, (float, int)) and math.isfinite(value) and -2 <= value <= 1000 for value in values)
        valid = valid and all(match.get('score', 0) >= parameters.get('threshold', 1) and match.get('amplitude', 0) >= .04 for match in matches)
        if not valid: problems.append(f"PCM markers missing, ambiguous or out of bounds: {edge.get('peer')}/{edge.get('source')}")
        else:
            all_values.extend(values)
            summaries.append({'peer': edge['peer'], 'source': edge['source'], 'markers': len(values), 'min_ms': min(values), 'median_ms': median(values), 'max_ms': max(values)})
    ordered = sorted(all_values)
    return {'scope': evidence.get('scope'), 'calibrated_detector_error_bound_ms': parameters.get('errorBoundMs'),
        'edges': summaries, 'markers': len(all_values), 'median_ms': median(all_values),
        'p95_ms': ordered[math.ceil(.95 * len(ordered)) - 1] if ordered else None,
        'max_ms': max(all_values) if all_values else None}


def summarize(browser, server):
    problems = list(browser.get('failures', []))
    if server.get('loadgen_returncode') != 0:
        problems.append('load generator failed')
    problems.extend(server.get('monitoring_errors', []))
    samples = browser.get('samples', [])
    result = {'problems': problems, 'media_fixture_valid': False,
              'fixed_video_fixture': browser.get('input', {}).get('fixedVideoFixture', False),
              'pcm_latency_enabled': browser.get('input', {}).get('pcmLatency', False),
              'executed_browser': browser.get('load_generator', {}).get('executed_browser', {})}
    if len(samples) < 3:
        problems.append('at least three browser samples required')
        return result
    first, last = samples[0], samples[-1]
    seconds = (last['at'] - first['at']) / 1000
    if seconds <= 0:
        problems.append('nonpositive browser measurement interval')
        return result
    initial = media_stats(first, 'inbound-rtp')
    incoming = media_stats(last, 'inbound-rtp')
    incoming = [s for s in incoming if s.get('packetsReceived', 0) > 0]
    count = browser['peers']
    expected = {'audio': count * (count - 1) + (count - 1 if browser['video'] else 0),
                'video': count - 1 if browser['video'] else 0}
    observed = {kind: sum(s.get('kind') == kind for s in incoming) for kind in expected}
    if observed != expected:
        problems.append(f'incomplete media graph: expected {expected}, observed {observed}')
    per_peer = {}
    for index in range(count):
        peer = f'peer-{index}'
        suffix = {'current': '0', 'mediasoup': 'recv', 'janus': '1'}[browser['backend']]
        endpoint = peer + '/' + suffix
        peer_expected = {'audio': count - 1 + (1 if browser['video'] and index != 0 else 0),
                         'video': 1 if browser['video'] and index != 0 else 0}
        peer_observed = {kind: sum(s.get('_peer') == peer and s.get('kind') == kind for s in incoming)
                         for kind in expected}
        per_peer[peer] = {'expected': peer_expected, 'observed': peer_observed, 'expected_endpoint': endpoint}
        if peer_observed != peer_expected:
            problems.append(f'peer media graph mismatch: {peer}')
        if any(s.get('_peer') == peer and s.get('_endpoint') != endpoint for s in incoming):
            problems.append(f'media arrived at unexpected peer endpoint: {peer}')
    fps = [frames / seconds for frames in differences(initial, incoming, 'framesDecoded', 'video')]
    if browser['video']:
        if len(fps) != expected['video'] or any(value < 57 for value in fps):
            problems.append('not every video receiver decoded at least 57 fps')
        if any(s.get('frameWidth') != 1920 or s.get('frameHeight') != 1080
               for s in incoming if s.get('kind') == 'video'):
            problems.append('not every video receiver decoded 1920x1080')
    for kind in ['audio', 'video']:
        packets = differences(initial, incoming, 'packetsReceived', kind)
        if len(packets) != expected[kind] or any(value <= 0 for value in packets):
            problems.append(f'{kind} media did not advance on every forwarding edge')
    sent_first, sent_last = media_stats(first, 'outbound-rtp'), media_stats(last, 'outbound-rtp')
    result['sender_bitrate_bps_per_stream'] = {
        kind: median([value * 8 / seconds for value in differences(sent_first, sent_last, 'bytesSent', kind)])
        for kind in expected}
    sent_before = {key(s): s for s in sent_first}
    rates = [{'peer': s.get('_peer'), 'endpoint': s.get('_endpoint'), 'id': s['id'], 'kind': s.get('kind'),
              'bps': (s['bytesSent'] - sent_before[key(s)]['bytesSent']) * 8 / seconds}
             for s in sent_last if key(s) in sent_before and 'bytesSent' in s
             and 'bytesSent' in sent_before[key(s)] and s['bytesSent'] >= sent_before[key(s)]['bytesSent']]
    result['sender_bitrate_distribution'] = {kind: {'streams': [s for s in rates if s['kind'] == kind],
        'min_bps': min((s['bps'] for s in rates if s['kind'] == kind), default=None),
        'max_bps': max((s['bps'] for s in rates if s['kind'] == kind), default=None)} for kind in expected}
    fixed_video = result['fixed_video_fixture']
    received_before = {key(s): s for s in initial}
    video_edges = [{'peer': s.get('_peer'), 'endpoint': s.get('_endpoint'), 'id': s['id'],
        'bps': (s['bytesReceived'] - received_before[key(s)]['bytesReceived']) * 8 / seconds}
        for s in incoming if s.get('kind') == 'video' and key(s) in received_before
        and 'bytesReceived' in s and 'bytesReceived' in received_before[key(s)]]
    result['receiver_video_distribution'] = video_edges
    if fixed_video:
        target = browser.get('input', {}).get('videoBitrate', 0)
        video_rates = result['sender_bitrate_distribution']['video']['streams']
        if not browser['video'] or target <= 0 or len(video_rates) != 1 or not .9 * target <= video_rates[0]['bps'] <= 1.1 * target:
            problems.append('fixed video source did not deliver its configured actual rate')
        if len(video_edges) != expected['video'] or any(not video_rates or not .9 * video_rates[0]['bps'] <= s['bps'] <= 1.1 * video_rates[0]['bps'] for s in video_edges):
            problems.append('fixed video rate did not arrive on every forwarding edge')
        executed = browser.get('load_generator', {}).get('executed_browser', {})
        if executed.get('product', '').split('/')[0] not in ['Chrome', 'HeadlessChrome'] or not executed.get('revision') or len(executed.get('sha256', '')) != 64:
            problems.append('fixed video fixture lacks executed Chromium provenance')
    # A graph can contain advancing counters while silently losing most audio.
    # Qualify every forwarding edge before treating its server work as equal.
    audio_edges = []
    for stream in incoming:
        if stream.get('kind') != 'audio': continue
        before = received_before.get(key(stream), {})
        if any(field not in stream or field not in before for field in ['bytesReceived', 'packetsReceived', 'packetsLost']):
            problems.append('missing per-edge audio delivery evidence'); continue
        received = stream['packetsReceived'] - before['packetsReceived']
        lost = max(0, stream['packetsLost'] - before['packetsLost'])
        audio_edges.append({'peer': stream.get('_peer'), 'endpoint': stream.get('_endpoint'), 'id': stream['id'],
            'bps': (stream['bytesReceived'] - before['bytesReceived']) * 8 / seconds,
            'packet_loss_fraction': lost / (received + lost) if received + lost > 0 else None})
    audio_rate = result['sender_bitrate_distribution']['audio']
    if not audio_rate['min_bps'] or len(audio_edges) != expected['audio']:
        problems.append('missing audio delivery rate comparison')
    elif any(edge['bps'] < audio_rate['min_bps'] * .9 or edge['bps'] > audio_rate['max_bps'] * 1.1
             or edge['packet_loss_fraction'] is None or edge['packet_loss_fraction'] > .01 for edge in audio_edges):
        problems.append('audio delivery rate/loss differs on a forwarding edge')
    result['receiver_audio_distribution'] = audio_edges
    result['per_peer_graph'] = per_peer
    timing = browser.get('join_timing', [])
    if len(timing) != count or any(any(field not in p for field in ['setup_started_at', 'dtls_ready_at', 'first_send_rtp_at', 'full_graph_rtp_ready_at']) for p in timing):
        problems.append('missing per-peer join timing evidence')
    else:
        result['join_timing_ms'] = [{'peer': p['peer'], **{field: p[field] - p['setup_started_at']
            for field in ['dtls_ready_at', 'first_send_rtp_at', 'full_graph_rtp_ready_at']}} for p in timing]
    post = server.get('post_leave_samples', [])
    if not post or not browser.get('post_leave'):
        problems.append('missing post-leave evidence')
    else:
        result['post_leave_peak_rss_bytes'] = max(s['rss_bytes'] for s in post)
        result['post_leave_engine_stats'] = browser['post_leave']
        counters = browser['post_leave'].get('engine_stats', {})
        if any(counters.get(field, 0) != 0 for field in ['peers', 'transports', 'producers', 'consumers']):
            problems.append('engine retained live peer resources after leave')
        if counters.get('rooms', 0) not in [0, []]:
            problems.append('engine retained rooms after leave')
    result.update({'expected_inbound': expected, 'observed_inbound': observed,
                   'decoded_video_fps': fps, 'measurement_seconds': seconds,
                   'receiver_jitter_seconds_median': median([s['jitter'] for s in incoming if 'jitter' in s]),
                   'latency_scope': 'RTP jitter only; end-to-end audio latency is not measured'})
    if result['pcm_latency_enabled']:
        result['pcm_latency'] = summarize_pcm(browser, problems)
        result['latency_scope'] = result['pcm_latency']['scope']
        if not seconds <= browser.get('pcm_latency', {}).get('sample_clock_seconds', 0) <= seconds + 1.3:
            problems.append('PCM sampleclock does not cover the RTP measurement window')
        concealment = []
        for stream in incoming:
            if stream.get('kind') != 'audio': continue
            before = received_before.get(key(stream), {})
            fields = ['concealedSamples', 'totalSamplesReceived']
            if any(field not in stream or field not in before for field in fields):
                problems.append('PCM measurement lacks per-edge decoder concealment evidence'); continue
            total = stream['totalSamplesReceived'] - before['totalSamplesReceived']
            concealed = stream['concealedSamples'] - before['concealedSamples']
            if total <= 0 or concealed < 0 or concealed / total > .01: problems.append('PCM audio decoder concealment exceeds one percent')
            concealment.append({'peer': stream.get('_peer'), 'id': stream['id'], 'fraction': concealed / total if total > 0 else None})
        result['pcm_latency']['decoder_concealment'] = concealment
    result['negotiated_ciphers'] = sorted({(s.get('dtlsCipher', ''), s.get('srtpCipher', ''))
                                           for s in last['stats'] if s['type'] == 'transport'})
    timed = [s for s in server.get('samples', []) if first['at'] / 1000 <= s['at'] <= last['at'] / 1000]
    idle = server.get('idle_samples', [])
    if len(timed) < 2 or not idle:
        problems.append('missing server samples aligned with browser interval')
    else:
        elapsed = timed[-1]['at'] - timed[0]['at']
        cpu_delta = timed[-1]['cpu_seconds'] - timed[0]['cpu_seconds']
        if elapsed <= 0 or cpu_delta < 0:
            problems.append('invalid server CPU counters')
        else:
            result['cpu_cores_mean'] = cpu_delta / elapsed
        result['idle_peak_rss_bytes'] = max(s['rss_bytes'] for s in idle)
        result['load_peak_rss_bytes'] = max(s['rss_bytes'] for s in timed)
        result['idle_ram_under_64_mib'] = result['idle_peak_rss_bytes'] <= 64 * 1024 ** 2
        result['load_ram_under_256_mib'] = result['load_peak_rss_bytes'] <= 256 * 1024 ** 2
        infrastructure = {}
        for name in {name for sample in timed for name in sample.get('infrastructure', {})}:
            values = [s['infrastructure'][name] for s in timed if name in s.get('infrastructure', {})]
            infrastructure[name] = {'peak_rss_bytes': max(v['rss_bytes'] for v in values), 'image_id': values[0].get('image_id'),
                'cpu_cores_mean': (values[-1]['cpu_seconds'] - values[0]['cpu_seconds']) / (values[-1]['at'] - values[0]['at'])
                if len(values) > 1 and values[-1]['at'] > values[0]['at'] else None}
            infrastructure[name]['idle_peak_rss_bytes'] = max((s['infrastructure'][name]['rss_bytes'] for s in idle if name in s.get('infrastructure', {})), default=None)
            infrastructure[name]['post_leave_peak_rss_bytes'] = max((s['infrastructure'][name]['rss_bytes'] for s in post if name in s.get('infrastructure', {})), default=None)
        result['infrastructure'] = infrastructure
    generator = [s for s in server.get('load_generator_samples', []) if first['at'] / 1000 <= s['at'] <= last['at'] / 1000]
    if generator:
        result['load_generator_peak_rss_bytes'] = max(s['rss_bytes'] for s in generator)
        stable = all(b['cpu_seconds'] >= a['cpu_seconds'] for a, b in zip(generator, generator[1:]))
        result['load_generator_cpu_counters_stable'] = stable
        if len(generator) >= 2 and stable and generator[-1]['at'] > generator[0]['at']:
            result['load_generator_cpu_cores_mean'] = (generator[-1]['cpu_seconds'] - generator[0]['cpu_seconds']) / (generator[-1]['at'] - generator[0]['at'])
    if result['pcm_latency_enabled'] and (not result.get('load_generator_cpu_counters_stable') or 'load_generator_cpu_cores_mean' not in result or not result.get('load_generator_peak_rss_bytes')):
        problems.append('PCM measurement lacks stable generator CPU/RAM evidence')
    result['media_fixture_valid'] = not problems
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    report = json.loads((args.directory / 'report.json').read_text())
    output = {'acceptance': False, 'blocking_gates': report['blocking_gates'],
              'source_revision': report['source_revision'],
              'comparison_disqualified': report.get('environment_disqualified') or report.get('error'),
              'runs': [], 'medians': []}
    for run in report['runs']:
        folder = args.directory / Path(run['directory']).name
        try:
            summary = summarize(json.loads((folder / 'browser.json').read_text()),
                                json.loads((folder / 'server.json').read_text()))
        except (OSError, ValueError, KeyError) as error:
            summary = {'media_fixture_valid': False, 'problems': [str(error)]}
        output['runs'].append({**run, **summary})
    for count in report['configuration']['matrix']:
        for engine in report['configuration']['engines']:
            runs = [r for r in output['runs'] if r['engine'] == engine and r['peers'] == count]
            valid = [r for r in runs if r['media_fixture_valid']]
            output['medians'].append({'engine': engine, 'peers': count, 'valid_runs': len(valid),
                'required_runs': max(3, report['configuration']['runs']),
                'cpu_cores': median([r['cpu_cores_mean'] for r in valid]),
                'idle_rss_bytes': median([r['idle_peak_rss_bytes'] for r in valid]),
                'load_rss_bytes': median([r['load_peak_rss_bytes'] for r in valid]),
                'sender_audio_bps': median([r['sender_bitrate_bps_per_stream']['audio'] for r in valid]),
                'sender_video_bps': median([r['sender_bitrate_bps_per_stream']['video'] for r in valid])
                if report['configuration']['video'] else None,
                'pcm_latency_median_ms': median([r['pcm_latency']['median_ms'] for r in valid if r.get('pcm_latency_enabled')]),
                'pcm_latency_p95_ms': median([r['pcm_latency']['p95_ms'] for r in valid if r.get('pcm_latency_enabled')])})
    # Input bitrate and actual decoder quality must be compared before CPU/RAM;
    # a smaller workload cannot qualify as a faster backend.
    for count in report['configuration']['matrix']:
        rows = [r for r in output['medians'] if r['peers'] == count]
        compared_runs = [run for run in output['runs'] if run['peers'] == count and run['media_fixture_valid']]
        same_browser = same_comparison_browser(compared_runs)
        comparable = not output['comparison_disqualified'] and all(r['valid_runs'] >= r['required_runs'] for r in rows) and len(rows) >= 2
        for field in ['sender_audio_bps', 'sender_video_bps']:
            values = [r[field] for r in rows]
            if field == 'sender_video_bps' and not report['configuration']['video']:
                continue
            comparable = comparable and all(v is not None and v > 0 for v in values)
            if comparable:
                comparable = max(values) / min(values) <= 1.1
        for row in rows:
            row['executed_browser_provenance_equal'] = same_browser
            row['equal_streams_comparison_available'] = comparable and same_browser
    (args.directory / 'summary.json').write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps(output['medians'], indent=2))
    expected_runs = len(report['configuration']['matrix']) * len(report['configuration']['engines']) * report['configuration']['runs']
    return 1 if len(output['runs']) != expected_runs or any(not r['media_fixture_valid'] for r in output['runs']) or output['comparison_disqualified'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
