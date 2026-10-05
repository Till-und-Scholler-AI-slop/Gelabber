#!/usr/bin/env python3
"""Local video-only SFU diagnostic; isolated native children/UUID containers.
Not a voice matrix, a remote acceptance run or a resource comparison.
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
from record import process_sample, process_tree, terminate_group

ROOT = Path(__file__).resolve().parent


def sample_tree(pid):
    processes = [process_sample(child) for child in process_tree(pid)]
    return {'at': time.time(), 'root_pid': pid, 'processes': processes,
            'rss_bytes': sum(p['rss_bytes'] for p in processes),
            'cpu_seconds': sum(p['cpu_seconds'] for p in processes)}


def resource_sample_valid(sample):
    processes = sample.get('processes', [])
    valid_processes = all(isinstance(p.get('pid'), int) and p['pid'] > 0
                          and math.isfinite(p.get('rss_bytes', float('nan'))) and p['rss_bytes'] >= 0
                          and math.isfinite(p.get('cpu_seconds', float('nan'))) and p['cpu_seconds'] >= 0 for p in processes)
    root_present = any(p.get('pid') == sample.get('root_pid') for p in processes)
    return bool(processes and valid_processes and root_present and sample.get('rss_bytes', 0) > 0
                and math.isfinite(sample.get('cpu_seconds', float('nan'))) and sample['cpu_seconds'] >= 0)


def resource_failures(result):
    failures = list(result.get('monitoring_errors', []))
    groups = {'backend': result.get('samples', []), 'generator': result.get('load_generator_samples', []),
              'post_leave': result.get('post_leave_samples', [])}
    if result.get('engine') == 'current':
        groups['redis'] = result.get('infrastructure_samples', [])
    for label, samples in groups.items():
        if len(samples) < 2:
            failures.append(label + ' samples missing')
        for sample in samples:
            if not resource_sample_valid(sample):
                failures.append(label + ' PID/RSS/CPU sample invalid'); break
    idle = result.get('idle_sample', {})
    if not resource_sample_valid(idle):
        failures.append('idle PID/RSS/CPU sample missing or invalid')
    return failures


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def case(args, engine):
    folder = args.output / engine
    folder.mkdir()
    name = 'gelabber-fixed-video-' + uuid.uuid4().hex[:12]
    token, http = secrets.token_hex(32), port()
    owned, child, generator, thread = [], None, None, None
    stop = threading.Event()
    samples, generators, infra, errors = [], [], [], []
    environment = {**os.environ, 'BENCH_TOKEN': token, 'BENCH_ADDR': f'127.0.0.1:{http}',
                   'MEDIA_ADDR': f'127.0.0.1:{http}', 'MEDIA_ICE_BIND': f'{args.media_ip}:0',
                   'BENCH_ADVERTISED_IP': args.media_ip, 'BENCH_UDP_MIN': str(args.udp_min),
                   'BENCH_UDP_MAX': str(args.udp_max)}
    image = None
    log = (folder / 'backend.log').open('w')
    result = {'engine': engine, 'scope': 'local one video publisher/decoder, zero voice peers',
              'comparison_available': False, 'production_feature_acceptance': False}
    try:
        redis_pid = None
        if engine == 'current':
            redis = name + '-redis'; owned.append(redis)
            docker('run', '-d', '--name', redis, '--label', 'gelabber.bench.fixed-video=' + name,
                   '-p', '127.0.0.1::6379', 'redis:8.10.1-alpine', 'redis-server', '--save', '', '--appendonly', 'no')
            info = json.loads(docker('inspect', redis))[0]
            redis_pid = info['State']['Pid']
            result['redis_image_id'] = info['Image']
            environment['REDIS_URL'] = 'redis://127.0.0.1:' + info['NetworkSettings']['Ports']['6379/tcp'][0]['HostPort']
        if engine == 'janus':
            image = docker('inspect', '--format', '{{.Id}}', 'gelabber-bench/janus:v1.4.2')
            config = folder / 'http.jcfg'
            config.write_text(f'general: {{ base_path="/janus"; http=true; port={http}; https=false; }}\nadmin: {{ admin_http=false; admin_https=false; }}\n')
            owned.append(name)
            docker('run', '-d', '--name', name, '--label', 'gelabber.bench.fixed-video=' + name, '--network', 'host',
                   '-v', f'{config}:/opt/janus/etc/janus/janus.transport.http.jcfg:ro', image,
                   '--interface=' + args.media_ip, '--nat-1-1=' + args.media_ip, '--ice-enforce-list=' + args.media_ip,
                   f'--rtp-port-range={args.udp_min}-{args.udp_max}', '--apisecret=' + token)
            pid = json.loads(docker('inspect', name))[0]['State']['Pid']
        else:
            executable = args.inputs / (engine + '-probe') / 'probe'
            child = subprocess.Popen([str(executable)], env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            pid = child.pid
        backend = f'http://127.0.0.1:{http}'
        for _ in range(150):
            if child and child.poll() is not None:
                raise RuntimeError('owned backend exited')
            try:
                with urllib.request.urlopen(backend + ('/janus/info' if engine == 'janus' else '/ready'), timeout=1):
                    break
            except OSError:
                time.sleep(.2)
        else:
            raise RuntimeError('owned backend readiness deadline')
        result['idle_sample'] = sample_tree(pid)

        def monitor():
            while not stop.wait(.5):
                try:
                    samples.append(sample_tree(pid))
                    if generator and generator.poll() is None:
                        generators.append(sample_tree(generator.pid))
                    if redis_pid:
                        infra.append(sample_tree(redis_pid))
                except OSError as error:
                    errors.append(str(error)); break
        thread = threading.Thread(target=monitor); thread.start()
        with (folder / 'loadgen.log').open('w') as output:
            generator = subprocess.Popen(['node', str(args.inputs / 'native-video-pilot.mjs'), '--engine', engine,
                '--backend', backend, '--binary', str(args.inputs / 'fixed-rtp-source'), '--archive', str(args.inputs / 'source.rtpbin'),
                '--bind', args.media_ip,
                '--seconds', str(args.seconds), '--warmup', str(args.warmup), '--output', str(folder / 'browser.json'), '--execute'],
                env=environment, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            generator.wait(timeout=args.seconds + args.warmup + 180)
        result['returncode'] = generator.returncode
        result['post_leave_samples'] = []
        for _ in range(6):
            time.sleep(.5); result['post_leave_samples'].append(sample_tree(pid))
    except Exception as error:
        result['error'] = str(error)
    finally:
        stop.set()
        if thread:
            thread.join()
        cleanup_errors = []
        for process in [generator, child]:
            try:
                terminate_group(process)
            except Exception as error:
                cleanup_errors.append(str(error))
        for container in reversed(owned):
            subprocess.run(['docker', 'logs', container], stdout=log, stderr=subprocess.STDOUT)
            try:
                labels = json.loads(docker('inspect', container))[0]['Config'].get('Labels') or {}
                if labels.get('gelabber.bench.fixed-video') != name:
                    raise RuntimeError('refusing cleanup of foreign container: ' + container)
                removed = subprocess.run(['docker', 'rm', '-f', container], capture_output=True, text=True)
                if removed.returncode:
                    cleanup_errors.append(removed.stderr.strip())
            except Exception as error:
                cleanup_errors.append(str(error))
        log.close()
        result.update(samples=samples, load_generator_samples=generators, infrastructure_samples=infra,
                      monitoring_errors=errors, janus_image_id=image, cleanup_errors=cleanup_errors,
                      owned_container_cleanup_valid=not cleanup_errors)
        result['resource_measurement_failures'] = resource_failures(result)
        result['resource_measurement_valid'] = not result['resource_measurement_failures']
        (folder / 'server.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--engines', nargs='+', choices=['current', 'mediasoup', 'janus'], default=['current', 'mediasoup', 'janus'])
    parser.add_argument('--seconds', type=int, default=20)
    parser.add_argument('--warmup', type=int, default=10)
    parser.add_argument('--udp-min', type=int, default=12000)
    parser.add_argument('--udp-max', type=int, default=12199)
    parser.add_argument('--media-ip', required=True, help='existing explicit IPv4 for native/med/Janus ICE; Janus skips loopback')
    args = parser.parse_args()
    try:
        media_ip = ipaddress.IPv4Address(args.media_ip)
    except ValueError:
        parser.error('existing explicit IPv4 required')
    if media_ip.is_unspecified or media_ip.is_multicast or ('janus' in args.engines and media_ip.is_loopback):
        parser.error('explicit nonloopback IPv4 required for Janus')
    if args.output.exists() or not args.binary.is_file() or not args.archive.is_file() or not 8 <= args.seconds <= 120 or not 0 <= args.warmup <= 60 or not 1024 <= args.udp_min < args.udp_max <= 65535:
        parser.error('fresh output, valid source files, seconds8..120/warmup0..60 and UDP range required')
    args.output = args.output.resolve(); args.inputs = args.output / 'inputs'; args.inputs.mkdir(parents=True)
    names = ['run-fixed-video.py', 'record.py', 'native-video-pilot.mjs', 'native-video-adapters.mjs', 'native-video-browser.mjs', 'native-video.bundle.js', 'native-video.mjs', 'mediasoup-native-sdp.mjs', 'janus-events.mjs', 'janus-broker.mjs', 'browser-provenance.mjs', 'proxy-target.mjs', 'package-lock.json']
    for name in names:
        shutil.copy2(ROOT / name, args.inputs / name)
    (args.inputs / 'node_modules').symlink_to(ROOT / 'node_modules', target_is_directory=True)
    shutil.copy2(args.binary, args.inputs / 'fixed-rtp-source'); shutil.copy2(args.archive, args.inputs / 'source.rtpbin')
    for engine in args.engines:
        if engine != 'janus':
            destination = args.inputs / (engine + '-probe'); destination.mkdir()
            shutil.copy2(ROOT / (engine + '-probe') / 'target/release' / ('gelabber-' + engine + '-probe'), destination / 'probe')
    report = {'acceptance': False, 'comparison_available': False, 'source_revision': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
              'source_dirty': bool(subprocess.check_output(['git', '-C', str(ROOT), 'status', '--porcelain'], text=True).strip()),
              'input_sha256': {str(p.relative_to(args.inputs)): hashlib.sha256(p.read_bytes()).hexdigest() for p in args.inputs.rglob('*') if p.is_file()}, 'runs': []}
    for engine in args.engines:
        print('Diagnostic ' + engine, flush=True); report['runs'].append(case(args, engine))
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return int(any(run.get('error') or run.get('returncode') or run.get('cleanup_errors') or not run['resource_measurement_valid'] for run in report['runs']))


if __name__ == '__main__':
    raise SystemExit(main())
