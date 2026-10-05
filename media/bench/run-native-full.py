#!/usr/bin/env python3
"""Frozen local full-N SFU pilot. No remote actions, product edits or rankings.

Default is a read-only plan. Execution uses UUID containers, one backend at a
time, an independent browser namespace, explicit quotas and owned-only cleanup.
"""
import argparse
import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import threading
import time
import urllib.request
import uuid
from record import process_sample, process_tree

ROOT = Path(__file__).resolve().parent
HELPERS = ['run-native-full.py', 'record.py', 'native-full-pilot.mjs', 'native-full-browser.mjs', 'native-full.bundle.js',
           'native-full-checks.mjs', 'native-peer-adapters.mjs', 'native-peer-current.mjs', 'mediasoup-native-peer-sdp.mjs',
           'native-peer-checks.mjs', 'native-peer.mjs', 'native-video.mjs', 'janus-events.mjs', 'janus-broker.mjs',
           'browser-provenance.mjs', 'proxy-target.mjs', 'package-lock.json']


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True, stderr=subprocess.STDOUT, timeout=120).strip()


def image(value):
    return json.loads(docker('image', 'inspect', value))[0]


def free_port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def phase(path):
    try:
        rows = path.read_text().splitlines()
        return json.loads(rows[-1])['phase'] if rows else 'starting'
    except (OSError, ValueError, KeyError):
        return 'starting'


def tree(pid, current_phase):
    processes = [process_sample(child) for child in process_tree(pid)]
    valid = bool(processes and any(value['pid'] == pid for value in processes) and all(
        value['rss_bytes'] >= 0 and math.isfinite(value['cpu_seconds']) and value['cpu_seconds'] >= 0 for value in processes))
    return {'at': time.time(), 'phase': current_phase, 'root_pid': pid, 'processes': processes,
            'rss_bytes': sum(value['rss_bytes'] for value in processes),
            'cpu_seconds': sum(value['cpu_seconds'] for value in processes), 'valid': valid}


def resources(samples, selected_phase='measurement', minimum_span=0):
    active = [sample for sample in samples if sample.get('phase') == selected_phase]
    span = active[-1]['at'] - active[0]['at'] if len(active) >= 2 else 0
    valid = len(active) >= 2 and span >= minimum_span and all(sample.get('valid') is True and sample.get('rss_bytes', 0) > 0 for sample in active)
    rates = [(right['cpu_seconds'] - left['cpu_seconds']) / (right['at'] - left['at'])
             for left, right in zip(active, active[1:]) if left.get('valid') is True and right.get('valid') is True and right['at'] > left['at']]
    if len(rates) != len(active) - 1 or any(rate < 0 or not math.isfinite(rate) for rate in rates):
        valid = False
    return {'measurement_valid': valid, 'phase': selected_phase, 'measurement_samples': len(active), 'actual_span_seconds': span,
            'maximum_process_rss_sum_bytes': max((sample.get('rss_bytes', 0) for sample in active), default=None),
            'cpu_core_equivalents': rates, 'scope': 'sum of actual process tree RSS and CPU; shared pages may be counted per process'}


def case(args, engine, plan):
    folder = args.output / engine
    folder.mkdir(mode=0o700)
    name = 'gelabber-native-full-' + uuid.uuid4().hex[:12]
    token, http_port = secrets.token_hex(32), free_port()
    media_ip = str(args.subnet.network_address + 2)
    backend = f'http://{media_ip}:{http_port}'
    owned, thread, cleanup_errors = [], None, []
    samples, generator_samples, infrastructure_samples, monitoring_errors = [], [], [], []
    stop = threading.Event()
    result = {'engine': engine, 'scope': plan['scope'], 'run_id': name, 'backend_image': plan['backend_images'][engine],
              'source_revision': plan['source_revision'], 'source_dirty': plan['source_dirty'], 'limits': plan['limits'],
              'comparison_available': False, 'production_feature_acceptance': False,
              'janus_product_adapter_inclusive_resource_qualification': False}
    label = 'gelabber.bench.native-full=' + name
    limits = ['--cpus=1', '--memory=512m', '--pids-limit=256']
    engine_image = plan['backend_images'][engine]['id']
    backend_pid, owned_network = None, None
    try:
        network = name + '-network'
        # Unchanged current webrtc mDNS joins INADDR_ANY. A Docker --internal
        # network has no default route and yields ENODEV before signaling.
        docker('network', 'create', '--subnet', str(args.subnet), '--label', label, network)
        owned_network = network
        env = {'BENCH_TOKEN': token, 'BENCH_ADDR': f'0.0.0.0:{http_port}', 'MEDIA_ADDR': f'0.0.0.0:{http_port}',
               'MEDIA_ICE_BIND': f'{media_ip}:0', 'BENCH_ADVERTISED_IP': media_ip,
               'BENCH_UDP_MIN': str(args.udp_min), 'BENCH_UDP_MAX': str(args.udp_max)}
        redis_pid = None
        if engine == 'current':
            redis_name, redis_port = name + '-redis', 6379
            owned.append(redis_name)
            docker('run', '-d', '--name', redis_name, '--label', label, '--network', network, '--ip', str(args.subnet.network_address + 4), '--cpus=.25', '--memory=64m', '--pids-limit=64',
                   plan['redis_image']['id'], 'redis-server', '--bind', '0.0.0.0', '--port', str(redis_port), '--save', '', '--appendonly', 'no')
            redis_info = json.loads(docker('inspect', redis_name))[0]
            redis_pid = redis_info['State']['Pid']
            result['infrastructure'] = {'redis_image': plan['redis_image'], 'limits': {'cpu': .25, 'memory_bytes': 67108864, 'pids': 64}}
            env['REDIS_URL'] = f'redis://{args.subnet.network_address + 4}:{redis_port}'
        backend_name = name + '-backend'
        owned.append(backend_name)
        command = ['run', '-d', '--name', backend_name, '--label', label, '--network', network, '--ip', media_ip, *limits]
        if engine == 'janus':
            config = folder / 'http.jcfg'
            config.write_text(f'general: {{ base_path="/janus"; http=true; port={http_port}; https=false; }}\nadmin: {{ admin_http=false; admin_https=false; }}\n')
            command += ['--mount', f'type=bind,src={config},dst=/opt/janus/etc/janus/janus.transport.http.jcfg,readonly', engine_image,
                        '--interface=' + media_ip, '--nat-1-1=' + media_ip, '--ice-enforce-list=' + media_ip,
                        f'--rtp-port-range={args.udp_min}-{args.udp_max}', '--apisecret=' + token]
        else:
            for key, value in env.items(): command += ['-e', key + '=' + value]
            command += [engine_image]
        docker(*command)
        backend_info = json.loads(docker('inspect', backend_name))[0]
        result['network'] = {'name': network, 'subnet': str(args.subnet), 'internal': False, 'backend': backend_info['NetworkSettings']['Networks'],
                             'policy': 'owned private Docker bridge and temporary host veth/routes; no published ports, host-network mode or manual host network configuration'}
        if backend_info['NetworkSettings']['Networks'][network]['IPAddress'] != media_ip: raise RuntimeError('actual backend interface differs from ICE bind')
        backend_pid = backend_info['State']['Pid']
        for _ in range(150):
            try:
                with urllib.request.urlopen(backend + ('/janus/info' if engine == 'janus' else '/ready'), timeout=1): break
            except OSError:
                if not json.loads(docker('inspect', backend_name))[0]['State']['Running']: raise RuntimeError('owned backend exited before readiness')
                time.sleep(.2)
        else: raise RuntimeError('owned backend readiness deadline')
        result['idle_sample'] = tree(backend_pid, 'idle')
        generator_name = name + '-generator'; owned.append(generator_name)
        phase_path = folder / 'phases.jsonl'
        common = ['run', '-d', '--name', generator_name, '--label', label, '--network', network, '--ip', str(args.subnet.network_address + 3), '--cpus=' + str(args.generator_cpus), '--memory=' + args.generator_memory,
                  '--pids-limit=512', '--read-only', '--tmpfs', '/tmp:rw,nosuid,nodev,size=256m', '--user', f'{os.getuid()}:{os.getgid()}',
                  '-e', 'BENCH_TOKEN=' + token, '--mount', f'type=bind,src={args.inputs},dst=/inputs,readonly',
                  '--mount', f'type=bind,src={folder},dst=/output', '--mount', f'type=bind,src={args.runtime},dst=/runtime,readonly',
                  '--mount', f'type=bind,src={ROOT / "node_modules"},dst={ROOT / "node_modules"},readonly',
                  '--mount', f'type=bind,src={args.browser_cache},dst=/chrome,readonly', plan['runtime_image']['id'],
                  '/fixture-bin/node', '/inputs/native-full-pilot.mjs', '--engine', engine, '--backend', backend,
                  '--binary', '/fixture-bin/native-peer', '--video', '/runtime/inputs/video.rtpbin', '--mic', '/runtime/controls/mic.opusbin',
                  '--source', '/runtime/controls/source.opusbin', '--chromium', '/chrome/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell',
                  '--peers', str(args.peers), '--seconds', str(args.seconds), '--warmup', str(args.warmup), '--source-seconds', str(args.source_seconds),
                  '--output', '/output/browser.json', '--phase', '/output/phases.jsonl', '--provenance', '/inputs/provenance.json', '--bind-interface', 'eth0', '--execute']
        docker(*common)
        generator_pid = json.loads(docker('inspect', generator_name))[0]['State']['Pid']

        def monitor():
            while not stop.wait(.5):
                current_phase = phase(phase_path)
                for label, pid, destination in [('backend', backend_pid, samples), ('generator', generator_pid, generator_samples), ('infrastructure', redis_pid, infrastructure_samples)]:
                    if not pid: continue
                    try: destination.append(tree(pid, current_phase))
                    except OSError as error:
                        destination.append({'at': time.time(), 'phase': current_phase, 'root_pid': pid, 'valid': False, 'error': str(error)})
                        if current_phase == 'measurement': monitoring_errors.append(label + ': ' + str(error))
        thread = threading.Thread(target=monitor); thread.start()
        waiting = subprocess.run(['docker', 'wait', generator_name], capture_output=True, text=True, timeout=args.source_seconds + 180)
        if waiting.returncode: raise RuntimeError('owned generator docker wait failed: ' + waiting.stderr)
        result['returncode'] = int(waiting.stdout.strip())
        result['generator_state'] = json.loads(docker('inspect', generator_name))[0]['State']
        result['post_leave_samples'] = []
        for _ in range(8): time.sleep(.5); result['post_leave_samples'].append(tree(backend_pid, 'post_leave'))
    except Exception as error:
        result['error'] = str(error)
    finally:
        stop.set()
        if thread: thread.join(timeout=5)
        for container in reversed(owned):
            try:
                info = json.loads(docker('inspect', container))[0]
                if (info['Config'].get('Labels') or {}).get('gelabber.bench.native-full') != name: raise RuntimeError('refusing cleanup of foreign container')
                (folder / (container.removeprefix(name + '-') + '.log')).write_text(docker('logs', container))
                docker('rm', '-f', container)
            except Exception as error: cleanup_errors.append(str(error))
        if owned_network:
            try:
                network_info = json.loads(docker('network', 'inspect', owned_network))[0]
                if (network_info.get('Labels') or {}).get('gelabber.bench.native-full') != name: raise RuntimeError('refusing cleanup of foreign network')
                docker('network', 'rm', owned_network)
            except Exception as error: cleanup_errors.append(str(error))
        result.update(samples=samples, load_generator_samples=generator_samples, infrastructure_samples=infrastructure_samples,
                      monitoring_errors=monitoring_errors, cleanup_errors=cleanup_errors, owned_cleanup_valid=not cleanup_errors,
                      backend_resources=resources(samples, minimum_span=args.seconds - 1), generator_resources=resources(generator_samples, minimum_span=args.seconds - 1),
                      infrastructure_resources=resources(infrastructure_samples, minimum_span=args.seconds - 1) if engine == 'current' else None,
                      post_leave_resources=resources(result.get('post_leave_samples', []), 'post_leave', 3))
        result['idle_resource_valid'] = result.get('idle_sample', {}).get('valid') is True and result.get('idle_sample', {}).get('rss_bytes', 0) > 0
        result['resource_measurement_valid'] = not monitoring_errors and result['backend_resources']['measurement_valid'] and result['generator_resources']['measurement_valid'] and (engine != 'current' or result['infrastructure_resources']['measurement_valid'])
        result['resource_lifetime_valid'] = result['idle_resource_valid'] and result['resource_measurement_valid'] and result['post_leave_resources']['measurement_valid']
        (folder / 'server.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=Path, required=True, help='frozen prepare-native-runtime output directory')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--subnet', default='172.30.232.0/24', help='unused private /24 for UUID Docker bridge; no published ports')
    parser.add_argument('--engines', nargs='+', choices=['current', 'mediasoup', 'janus'], default=['current', 'mediasoup', 'janus'])
    parser.add_argument('--peers', type=int, choices=[2, 8, 16, 32], default=2)
    parser.add_argument('--seconds', type=int, default=20)
    parser.add_argument('--warmup', type=int, default=10)
    parser.add_argument('--source-seconds', type=int, default=120)
    parser.add_argument('--udp-min', type=int, default=12000); parser.add_argument('--udp-max', type=int, default=12199)
    parser.add_argument('--generator-cpus', type=int, default=4)
    parser.add_argument('--generator-memory', default='4g', choices=['4g', '8g', '16g'])
    parser.add_argument('--browser-cache', type=Path, default=Path.home() / '.cache/ms-playwright')
    parser.add_argument('--current-image', default='gelabber-bench/current:debian13')
    parser.add_argument('--mediasoup-image', default='gelabber-bench/mediasoup:debian13')
    parser.add_argument('--janus-image', default='gelabber-bench/janus:v1.4.2')
    parser.add_argument('--redis-image', default='redis:8.10.1-alpine')
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    args.subnet = ipaddress.IPv4Network(args.subnet)
    if not args.subnet.is_private or args.subnet.prefixlen != 24 or args.subnet.network_address.is_loopback or not 1024 <= args.udp_min < args.udp_max <= 65535 or not 10 <= args.seconds <= 120 or not 5 <= args.warmup <= 60 or args.source_seconds % 10 or not args.seconds + args.warmup + 30 <= args.source_seconds <= 360 or args.generator_cpus < 1 or args.generator_cpus > 16:
        parser.error('unused private /24, bounded durations/UDP range and generator CPU1..16 required')
    for network_id in docker('network', 'ls', '-q').splitlines():
        info = json.loads(docker('network', 'inspect', network_id))[0]
        for item in info.get('IPAM', {}).get('Config') or []:
            if item.get('Subnet') and args.subnet.overlaps(ipaddress.ip_network(item['Subnet'])): parser.error('private subnet overlaps existing Docker network')
    for line in Path('/proc/net/route').read_text().splitlines()[1:]:
        fields = line.split()
        if fields[7] == '00000000': continue
        address = socket.inet_ntoa(bytes.fromhex(fields[1])[::-1]); mask = socket.inet_ntoa(bytes.fromhex(fields[7])[::-1])
        if args.subnet.overlaps(ipaddress.IPv4Network(address + '/' + mask)): parser.error('private subnet overlaps an existing host route')
    args.runtime = args.runtime.resolve(); args.output = args.output.resolve(); args.inputs = args.output / 'inputs'
    runtime = json.loads((args.runtime / 'report.json').read_text())
    if not runtime.get('offline_controls_valid') or not runtime.get('cleanup_valid') or not runtime.get('retained_image'): parser.error('valid retained native runtime required')
    runtime_image = image(runtime['image_id'])
    if runtime_image['Id'] != runtime['image_id'] or runtime_image['Config'].get('Labels', {}).get('gelabber.bench.native.sha256') != runtime['input_hashes']['native-peer']: parser.error('runtime image identity differs')
    backend_images = {engine: image(getattr(args, engine + '_image')) for engine in args.engines}
    def identity(value): return {'id': value['Id'], 'labels': value['Config'].get('Labels'), 'size': value['Size'], 'repo_digests': value.get('RepoDigests')}
    plan = {'schema': 1, 'scope': 'local full original N-participant native0 fixture; no independent-host WAN/PCM/product/performance acceptance',
            'execute': args.execute, 'source_revision': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
            'source_dirty': bool(subprocess.check_output(['git', '-C', str(ROOT), 'status', '--porcelain'], text=True).strip()),
            'peers': args.peers, 'video_watchers': args.peers - 1, 'audio_edges': args.peers ** 2 - 1, 'video_edges': args.peers - 1,
            'private_subnet': str(args.subnet), 'published_ports': [], 'udp_range': [args.udp_min, args.udp_max], 'seconds': args.seconds, 'warmup': args.warmup, 'source_seconds': args.source_seconds,
            'limits': {'backend': {'cpus': 1, 'memory_bytes': 536870912, 'pids': 256}, 'generator': {'cpus': args.generator_cpus, 'memory': args.generator_memory, 'pids': 512}, 'infrastructure_separate': True},
            'runtime_image': identity(runtime_image), 'runtime_report_sha256': hashlib.sha256((args.runtime / 'report.json').read_bytes()).hexdigest(),
            'backend_images': {engine: identity(value) for engine, value in backend_images.items()}, 'redis_image': identity(image(args.redis_image)),
            'comparison_available': False, 'production_feature_acceptance': False,
            'limitations': ['local engine and generator on one physical host', 'current cached backend revision is reported separately from collector source; it may precede new layer/recovery changes', 'Janus lacks the product ticket/ACL gateway; C-server resources alone cannot qualify its required adapter-inclusive RAM target']}
    if not args.execute: print(json.dumps(plan, indent=2)); return 0
    if args.output.exists(): parser.error('fresh output directory required')
    args.inputs.mkdir(parents=True, mode=0o700)
    for name in HELPERS: shutil.copy2(ROOT / name, args.inputs / name)
    (args.inputs / 'node_modules').symlink_to(ROOT / 'node_modules', target_is_directory=True)
    plan['input_sha256'] = {name: hashlib.sha256((args.inputs / name).read_bytes()).hexdigest() for name in HELPERS}
    (args.inputs / 'provenance.json').write_text(json.dumps(plan, indent=2) + '\n')
    plan['runs'] = []
    for engine in args.engines:
        print('Full-native pilot ' + engine, flush=True); plan['runs'].append(case(args, engine, plan))
        (args.output / 'report.json').write_text(json.dumps(plan, indent=2) + '\n')
    return int(any(run.get('error') or run.get('returncode') or not run['owned_cleanup_valid'] or not run['resource_lifetime_valid'] for run in plan['runs']))


if __name__ == '__main__':
    raise SystemExit(main())
