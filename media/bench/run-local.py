#!/usr/bin/env python3
"""Informational local probes; acceptance requires a separate load-generator host.
Owns only fresh native children and UUID-named test Redis/Janus containers.
"""
import argparse
import hashlib
import json
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
from record import hardware, process_sample, process_tree, terminate_group

ROOT = Path(__file__).resolve().parent


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def free_port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def wait_http(url, child=None):
    for _ in range(150):
        if child is not None and child.poll() is not None:
            raise RuntimeError('owned backend exited before readiness')
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if response.status == 200:
                    return
        except (OSError, TimeoutError):
            time.sleep(.2)
    raise RuntimeError('backend readiness timed out')


def sample_tree(pid):
    children = [process_sample(value) for value in process_tree(pid)]
    return {'at': time.time(), 'processes': children, 'rss_bytes': sum(p['rss_bytes'] for p in children), 'cpu_seconds': sum(p['cpu_seconds'] for p in children)}


def run(args, engine, count, round_number, report):
    name = 'gelabber-bench-' + uuid.uuid4().hex[:12]
    token, port = secrets.token_hex(32), free_port()
    folder = args.output / f'{engine}-{count}-{round_number}'
    folder.mkdir(parents=True)
    containers, child, loadgen, thread, stop = [], None, None, None, threading.Event()
    log = (folder / 'backend.log').open('w')
    try:
        environment = {**os.environ, 'BENCH_TOKEN': token, 'BENCH_ADDR': f'127.0.0.1:{port}', 'MEDIA_ADDR': f'127.0.0.1:{port}', 'BENCH_ADVERTISED_IP': '127.0.0.1', 'MEDIA_ICE_BIND': '127.0.0.1:0'}
        if engine == 'current':
            redis_name = name + '-redis'
            containers.append(redis_name)
            docker('run', '-d', '--name', redis_name, '-p', '127.0.0.1::6379', 'redis:8.10.1-alpine', 'redis-server', '--save', '', '--appendonly', 'no')
            redis_port = json.loads(docker('inspect', redis_name))[0]['NetworkSettings']['Ports']['6379/tcp'][0]['HostPort']
            environment['REDIS_URL'] = f'redis://127.0.0.1:{redis_port}'
            binary = args.snapshot / 'current-probe/target/release/gelabber-current-probe'
            child = subprocess.Popen([str(binary)], env=environment, stdout=log, stderr=subprocess.STDOUT)
            pid = child.pid
        elif engine == 'mediasoup':
            binary = args.snapshot / 'mediasoup-probe/target/release/gelabber-mediasoup-probe'
            child = subprocess.Popen([str(binary)], env=environment, stdout=log, stderr=subprocess.STDOUT)
            pid = child.pid
        else:
            config = folder / 'http.jcfg'
            config.write_text(f'general: {{ json = "compact"; base_path = "/janus"; http = true; port = {port}; https = false; }}\nadmin: {{ admin_http = false; admin_https = false; }}\n')
            containers.append(name)
            docker('run', '-d', '--name', name, '--network', 'host', '-v', f'{config.resolve()}:/opt/janus/etc/janus/janus.transport.http.jcfg:ro', args.janus_image, '--interface=127.0.0.1', '--nat-1-1=127.0.0.1', '--apisecret=' + token)
            inspected = json.loads(docker('inspect', name))[0]
            pid = inspected['State']['Pid']
            report['janus_image_id'] = inspected['Image']
        backend = f'http://127.0.0.1:{port}'
        wait_http(backend + ('/janus/info' if engine == 'janus' else '/ready'), child)
        time.sleep(1)
        idle = []
        for _ in range(3):
            idle.append(sample_tree(pid))
            time.sleep(.5)
        samples, monitoring_errors = [], []
        def monitor():
            while not stop.wait(.5):
                try:
                    samples.append(sample_tree(pid))
                except (OSError, ProcessLookupError) as error:
                    monitoring_errors.append(str(error))
                    break
        thread = threading.Thread(target=monitor)
        thread.start()
        browser_file = folder / 'browser.json'
        with (folder / 'loadgen.log').open('w') as output:
            loadgen = subprocess.Popen(['node', str(args.snapshot / 'loadgen.mjs'), '--engine', engine, '--backend', backend, '--peers', str(count), '--video', str(args.video).lower(), '--video-bitrate', str(args.video_bitrate), '--protocol-logs', str(args.protocol_logs).lower(), '--warmup', str(args.warmup * 1000), '--duration', str(args.duration * 1000), '--output', str(browser_file)], env=environment, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            loadgen.wait(timeout=args.duration + args.warmup + 300)
        post_leave = []
        for _ in range(6):
            time.sleep(.5)
            post_leave.append(sample_tree(pid))
        stop.set()
        thread.join()
        metrics = {'schema': 1, 'server_hostname': socket.gethostname(), 'engine': engine, 'peers': count, 'round': round_number, 'idle_samples': idle, 'post_leave_samples': post_leave, 'samples': samples, 'monitoring_errors': monitoring_errors, 'loadgen_returncode': loadgen.returncode, 'scope': 'local informational probe; shared Redis excluded from media-only RAM'}
        (folder / 'server.json').write_text(json.dumps(metrics, indent=2) + '\n')
        report['runs'].append({'engine': engine, 'peers': count, 'round': round_number, 'directory': str(folder), 'loadgen_returncode': loadgen.returncode, 'idle_peak_rss_bytes': max(s['rss_bytes'] for s in idle), 'observed_peak_rss_bytes': max((s['rss_bytes'] for s in samples), default=0)})

    finally:
        stop.set()
        if thread:
            thread.join(timeout=5)
        terminate_group(loadgen)
        if child:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        for container in reversed(containers):
            subprocess.run(['docker', 'logs', container], stdout=log, stderr=subprocess.STDOUT)
            subprocess.run(['docker', 'rm', '-f', '-v', container], capture_output=True)
        log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engines', nargs='+', choices=['current', 'mediasoup', 'janus'], default=['current', 'mediasoup', 'janus'])
    parser.add_argument('--matrix', nargs='+', type=int, choices=[2, 8, 16, 32], default=[2, 8, 16, 32])
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--duration', type=int, default=60)
    parser.add_argument('--warmup', type=int, default=10)
    parser.add_argument('--video', action='store_true')
    parser.add_argument('--video-bitrate', type=int, default=6000000)
    parser.add_argument('--protocol-logs', action='store_true', help='Chromium RTC event logs; diagnostic runs only')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 1 or args.duration < 3 or args.warmup < 1 or not 0 < args.video_bitrate <= 100000000:
        parser.error('runs >=1, duration >=3 and warmup >=1 required')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    args.janus_image = docker('inspect', '--format', '{{.Id}}', 'gelabber-bench/janus:v1.4.2') if 'janus' in args.engines else None
    artifacts = ['client.bundle.js', 'package-lock.json', 'current-probe/Cargo.lock', 'mediasoup-probe/Cargo.lock', 'current-probe/target/release/gelabber-current-probe', 'mediasoup-probe/target/release/gelabber-mediasoup-probe']
    args.snapshot = args.output / 'inputs'
    for name in [*artifacts, 'loadgen.mjs', 'proxy-target.mjs']:
        source = ROOT / name
        if source.exists():
            destination = args.snapshot / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    (args.snapshot / 'node_modules').symlink_to(ROOT / 'node_modules', target_is_directory=True)
    report = {'schema': 1, 'acceptance': False, 'blocking_gates': ['separate load-generator host', 'WAN/TURN', 'product tickets/ACL/revocation/watch/source-audio acceptance', 'real end-to-end audio latency'], 'server_hostname': socket.gethostname(), 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), 'configuration': {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()}, 'runs': []}
    report['artifact_sha256'] = {name: hashlib.sha256((args.snapshot / name).read_bytes()).hexdigest() for name in artifacts if (args.snapshot / name).exists()}
    report['server_hardware'] = hardware()
    report['tooling_sha256'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in ROOT.rglob('*') if path.is_file() and not any(part in ['node_modules', 'target', '__pycache__', 'artifacts'] for part in path.relative_to(ROOT).parts)}
    try:
        for count in args.matrix:
            for round_number in range(1, args.runs + 1):
                engines = args.engines if round_number % 2 else list(reversed(args.engines))
                for engine in engines:
                    print(f'Probe {engine}: {count} peers, round {round_number}', flush=True)
                    run(args, engine, count, round_number, report)
                    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    except (Exception, KeyboardInterrupt) as error:
        report['error'] = str(error) or type(error).__name__
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 1 if 'error' in report or any(run['loadgen_returncode'] for run in report['runs']) else 0


if __name__ == '__main__':
    raise SystemExit(main())
