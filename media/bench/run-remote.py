#!/usr/bin/env python3
"""Separate local browser generator + explicitly selected remote Docker server.
Without --execute, prints a concrete plan and performs no remote operations.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import subprocess
import threading
import time
import urllib.request
import uuid
from record import hardware, process_sample, process_tree, terminate_group

ROOT = Path(__file__).resolve().parent
REMOTE = r'''
import json,sys,subprocess,time
from pathlib import Path
c=json.load(sys.stdin)
group=c['group']; assert group.startswith('gelabber-bench-') and group.removeprefix('gelabber-bench-').isalnum()
def docker(*args): return subprocess.check_output(['docker',*args],text=True).strip()
def owned(name):
    assert name in [group+'-engine',group+'-redis']
    p=subprocess.run(['docker','inspect',name],capture_output=True,text=True)
    if p.returncode: return None
    value=json.loads(p.stdout)[0]
    assert value['Config']['Labels'].get('gelabber.bench.run')==group, 'refusing foreign container'
    return value
op=c['op']; name=group+'-engine'; redis_name=group+'-redis'
if op=='start':
    assert not owned(name) and not owned(redis_name)
    env={'BENCH_TOKEN':c['token'],'BENCH_ADDR':f"0.0.0.0:{c['port']}",'MEDIA_ADDR':f"0.0.0.0:{c['port']}",
         'BENCH_ADVERTISED_IP':c['ip'],'MEDIA_ADVERTISED_IP':c['ip'],
         'BENCH_UDP_MIN':str(c['udp_min']),'BENCH_UDP_MAX':str(c['udp_max']),
         'MEDIA_ICE_BIND':f"0.0.0.0:{c['udp_min']}",'MEDIA_ICE_PORT_MAX':str(c['udp_max'])}
    if c['engine']=='current':
        docker('run','-d','--name',redis_name,'--label','gelabber.bench.run='+group,'--label','com.centurylinklabs.watchtower.enable=false','--cpus','0.25','--memory','64m','--memory-swap','64m','--pids-limit','64',
               '-p','127.0.0.1::6379','redis:8.10.1-alpine','redis-server','--save','','--appendonly','no')
        port=owned(redis_name)['NetworkSettings']['Ports']['6379/tcp'][0]['HostPort']
        env['REDIS_URL']='redis://127.0.0.1:'+port
    args=['run','-d','--name',name,'--label','gelabber.bench.run='+group,'--label','com.centurylinklabs.watchtower.enable=false','--network','host',
          '--cpus','1','--memory','512m','--memory-swap','512m','--pids-limit','256','--log-opt','max-size=5m','--log-opt','max-file=1']
    if c['engine']=='janus': env['BENCH_HTTP_PORT']=str(c['port'])
    for key,value in env.items(): args+=['-e',key+'='+value]
    args+=[c['image']]
    if c['engine']=='janus': args+=['--interface='+c['ip'],'--nat-1-1='+c['ip'],'--rtp-port-range='+str(c['udp_min'])+'-'+str(c['udp_max']),'--apisecret='+c['token']]
    docker(*args)
    info=owned(name)
    ns={'__name__':'bench_record'}; exec(c['record_source'],ns)
    support=owned(redis_name)
    print(json.dumps({'pid':info['State']['Pid'],'image_id':info['Image'],'at':time.time(),'hardware':ns['hardware'](),
                      'redis_image_id':support['Image'] if support else None}))
elif op=='clock':
    print(json.dumps({'at':time.time()}))
elif op=='sample':
    info=owned(name); assert info and info['State']['Running'], 'owned media process exited'
    ns={'__name__':'bench_record'}; exec(c['record_source'],ns)
    def measure(info):
        pid=info['State']['Pid']; processes=[ns['process_sample'](p)for p in ns['process_tree'](pid)]
        value={'at':time.time(),'processes':processes,'rss_bytes':sum(p['rss_bytes']for p in processes),'cpu_seconds':sum(p['cpu_seconds']for p in processes)}
        groups=Path(f'/proc/{pid}/cgroup').read_text().splitlines()
        v2=next((line.split('::',1)[1]for line in groups if line.startswith('0::')),None)
        if v2:
            cg=Path('/sys/fs/cgroup')/v2.lstrip('/')
            value['cgroup_memory_current']=int((cg/'memory.current').read_text())
            value['cgroup_cpu_stat']=(cg/'cpu.stat').read_text()
        return value
    value=measure(info); support=owned(redis_name)
    if support: value['infrastructure']={'redis':{**measure(support),'image_id':support['Image']}}
    print(json.dumps(value))
elif op=='cleanup':
    logs={}
    for n in [name,redis_name]:
        info=owned(n)
        if info:
            logs[n]={'logs':docker('logs',n).replace(c['token'],'[redacted]'),
                     'state':{key:info['State'][key]for key in ['Status','ExitCode','OOMKilled','StartedAt','FinishedAt']}}
            docker('rm','-f','-v',n)
    print(json.dumps(logs))
elif op=='images_cleanup':
    errors=[]
    for tag in c['tags']:
        assert tag.startswith('gelabber-bench/'+group+'-')
        outcome=subprocess.run(['docker','image','rm',tag],capture_output=True,text=True)
        if outcome.returncode and 'No such image' not in outcome.stderr: errors.append(outcome.stderr)
    print(json.dumps({'errors':errors}))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--ssh-control', required=True)
    parser.add_argument('--server-ip', required=True)
    parser.add_argument('--port', type=int, default=18091)
    parser.add_argument('--udp-min', type=int, default=11000)
    parser.add_argument('--udp-max', type=int, default=11199)
    parser.add_argument('--engines', nargs='+', choices=['current', 'mediasoup', 'janus'], default=['current', 'mediasoup', 'janus'])
    parser.add_argument('--matrix', nargs='+', type=int, choices=[2, 8, 16, 32], default=[2, 8, 16, 32])
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--duration', type=int, default=60)
    parser.add_argument('--warmup', type=int, default=10)
    parser.add_argument('--video', action='store_true')
    parser.add_argument('--video-bitrate', type=int, default=6000000)
    parser.add_argument('--fixed-video-fixture', action='store_true', help='identical Chrome min/start/max source hints; actual bitrate/FPS still required')
    parser.add_argument('--pcm-calibration', type=Path, help='enable PCM marker latency with a passing local calibration JSON')
    parser.add_argument('--protocol-logs', action='store_true', help='Chromium RTC event logs; diagnostic runs only')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    ipaddress.ip_address(args.server_ip)
    if args.runs < 1 or args.duration < 3 or args.warmup < 1 or not 0 < args.video_bitrate <= 100000000 or not (1024 <= args.port <= 65535 and 1024 <= args.udp_min <= args.udp_max <= 65535):
        parser.error('invalid matrix duration or port range')
    if args.fixed_video_fixture and (not args.video or args.video_bitrate % 1000):
        parser.error('fixed video fixture requires --video and whole kbit/s')
    if args.pcm_calibration and (not args.pcm_calibration.is_file() or args.duration < 8):
        parser.error('PCM calibration file and duration >=8 required')
    group = 'gelabber-bench-' + uuid.uuid4().hex[:12]
    original = {engine: ('gelabber-bench/janus:v1.4.2' if engine == 'janus' else f'gelabber-bench/{engine}:debian13') for engine in args.engines}
    images = {engine: json.loads(subprocess.check_output(['docker', 'image', 'inspect', image], text=True))[0] for engine, image in original.items()}
    if sum(image['Size'] for image in images.values()) > 1536 * 1024 ** 2:
        parser.error('image-size upper bound exceeds the shared-host 1.5 GiB transfer limit')
    tags = {engine: 'gelabber-bench/' + group + '-' + engine + ':owned' for engine in args.engines}
    report = {'schema': 1, 'acceptance': False, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'blocking_gates': ['production tickets/ACL/revocation/watch/source-audio', 'TURN/loss/browser-family acceptance', 'true end-to-end audio latency', 'shared production host interference'],
        'server_hostname': args.ssh_host, 'server_ip': args.server_ip, 'configuration': {key: str(v) if isinstance(v, Path) else v for key, v in vars(args).items()},
        'limits': {'media_cpus': 1, 'memory_bytes': 512 * 1024**2, 'pids': 256, 'redis_cpus': .25, 'redis_memory_bytes': 64 * 1024**2, 'cohosted_live_services': True},
        'git_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip()),
        'images': {engine: {'id': image['Id'], 'size': image['Size'], 'labels': image['Config']['Labels'], 'owned_tag': tags[engine]} for engine, image in images.items()}, 'runs': []}
    if not args.execute:
        print(json.dumps(report, indent=2)); return 0
    args.output = args.output.resolve(); args.output.mkdir(parents=True)
    snapshot = args.output / 'inputs'; snapshot.mkdir()
    for filename in ['loadgen.mjs', 'janus-broker.mjs', 'janus-events.mjs', 'proxy-target.mjs', 'browser-provenance.mjs', 'video-fixture.mjs', 'pcm-policy.mjs', 'pcm-kernel.mjs', 'pcm-marker.mjs', 'pcm.bundle.js', 'client.bundle.js', 'package-lock.json', 'record.py', 'run-remote.py', 'evaluate.py', 'protocol-diagnostics.py']:
        shutil.copy2(ROOT / filename, snapshot / filename)
    if args.pcm_calibration:
        shutil.copy2(args.pcm_calibration, snapshot / 'pcm-calibration.json')
    (snapshot / 'remote-helper.py').write_text(REMOTE)
    (snapshot / 'node_modules').symlink_to(ROOT / 'node_modules', target_is_directory=True)
    report['artifact_sha256'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in snapshot.iterdir() if p.is_file()}
    report['load_generator_hardware'] = hardware()
    collector_source = (snapshot / 'record.py').read_text()
    remote_source = (snapshot / 'remote-helper.py').read_text()
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'ControlMaster=no', '-S', args.ssh_control, args.ssh_host]
    def remote(payload):
        encoded = {**payload, 'group': group, 'record_source': collector_source}
        text = subprocess.check_output([*ssh, shlex.join(['python3', '-c', remote_source])], input=json.dumps(encoded), text=True, timeout=30)
        return json.loads(text)
    def save(): (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    imported, exporter, loader = False, None, None
    try:
        for engine, image in images.items(): subprocess.run(['docker', 'tag', image['Id'], tags[engine]], check=True)
        exporter = subprocess.Popen(['docker', 'save', *tags.values()], stdout=subprocess.PIPE)
        imported = True  # Cleanup unique tags even if load is interrupted.
        loader = subprocess.Popen([*ssh, 'docker load'], stdin=exporter.stdout, stdout=subprocess.PIPE, text=True)
        exporter.stdout.close()
        loader.communicate(timeout=180)
        if loader.returncode or exporter.wait(timeout=30): raise RuntimeError('owned image transfer failed')
        token = secrets.token_hex(32)
        payload = {'port': args.port, 'udp_min': args.udp_min, 'udp_max': args.udp_max, 'ip': args.server_ip, 'token': token}
        for count in args.matrix:
            for round_number in range(1, args.runs + 1):
                for engine in args.engines if round_number % 2 else list(reversed(args.engines)):
                    folder = args.output / f'{engine}-{count}-{round_number}'; folder.mkdir()
                    conf = {**payload, 'engine': engine, 'image': tags[engine]}
                    child, thread, stop, samples, generator_samples, errors = None, None, threading.Event(), [], [], []
                    try:
                        print(f'Remote {engine}: {count} peers, round {round_number}', flush=True)
                        started = remote({**conf, 'op': 'start'})
                        report['server_hardware'] = started['hardware']
                        before = time.time(); clock = remote({**conf, 'op': 'clock'}); after = time.time()
                        offset = clock['at'] - (before + after) / 2
                        backend = f'http://{args.server_ip}:{args.port}'
                        for attempt in range(150):
                            try:
                                with urllib.request.urlopen(backend + ('/janus/info' if engine == 'janus' else '/ready'), timeout=1): break
                            except OSError: time.sleep(.2)
                        else: raise RuntimeError('dedicated backend readiness timed out')
                        def sample():
                            value = remote({**conf, 'op': 'sample'}); value['server_at'] = value['at']; value['at'] -= offset; return value
                        time.sleep(1)
                        idle = []
                        for _ in range(3): idle.append(sample()); time.sleep(.5)
                        def monitor():
                            while not stop.wait(.5):
                                try:
                                    samples.append(sample())
                                    if child and child.poll() is None:
                                        try:
                                            processes = [process_sample(p) for p in process_tree(child.pid)]
                                            generator_samples.append({'at': time.time(), 'processes': processes,
                                                'rss_bytes': sum(p['rss_bytes'] for p in processes), 'cpu_seconds': sum(p['cpu_seconds'] for p in processes)})
                                        except OSError: pass  # Browser children may finish after leave.
                                except Exception as error: errors.append(str(error)); break
                        thread = threading.Thread(target=monitor); thread.start()
                        with (folder / 'loadgen.log').open('w') as log:
                            child = subprocess.Popen(['node', str(snapshot / 'loadgen.mjs'), '--engine', engine, '--backend', backend, '--peers', str(count), '--video', str(args.video).lower(),
                                '--video-bitrate', str(args.video_bitrate), '--fixed-video-fixture', str(args.fixed_video_fixture).lower(), '--pcm-latency', str(bool(args.pcm_calibration)).lower(), '--pcm-calibration', str(snapshot / 'pcm-calibration.json'), '--protocol-logs', str(args.protocol_logs).lower(), '--warmup', str(args.warmup * 1000), '--duration', str(args.duration * 1000), '--separate-host', 'true', '--output', str(folder / 'browser.json')],
                                env={**os.environ, 'BENCH_TOKEN': token}, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                            child.wait(timeout=args.duration + args.warmup + 300)
                        post = []
                        for _ in range(6): time.sleep(.5); post.append(sample())
                        stop.set(); thread.join()
                        data = {'schema': 1, 'engine': engine, 'peers': count, 'round': round_number, 'server_hostname': args.ssh_host, 'idle_samples': idle, 'samples': samples,
                                'post_leave_samples': post, 'monitoring_errors': errors, 'loadgen_returncode': child.returncode, 'image_id': started['image_id'], 'clock_offset_estimate_seconds': offset,
                                'clock_round_trip_seconds': after - before, 'limits': report['limits']}
                        data['redis_image_id'] = started['redis_image_id']
                        data['load_generator_samples'] = generator_samples
                        (folder / 'server.json').write_text(json.dumps(data, indent=2) + '\n')
                        report['runs'].append({'engine': engine, 'peers': count, 'round': round_number, 'directory': str(folder), 'loadgen_returncode': child.returncode})
                    finally:
                        stop.set()
                        if thread: thread.join(timeout=35)
                        terminate_group(child)
                        logs = remote({**conf, 'op': 'cleanup'})
                        (folder / 'backend.log').write_text(json.dumps(logs, indent=2) + '\n')
                    save()
    except (Exception, KeyboardInterrupt) as error:
        report['error'] = str(error) or type(error).__name__
    finally:
        for transfer in [loader, exporter]:
            if transfer and transfer.poll() is None:
                transfer.terminate()
                try: transfer.wait(timeout=5)
                except subprocess.TimeoutExpired: transfer.kill(); transfer.wait()
        if imported:
            try:
                cleanup = remote({'op': 'images_cleanup', 'tags': list(tags.values())})
                if cleanup['errors']: report['images_cleanup_error'] = cleanup['errors']
            except Exception as error: report['images_cleanup_error'] = str(error)
        for tag in tags.values(): subprocess.run(['docker', 'image', 'rm', tag], capture_output=True)
        save()
    return 1 if 'error' in report or any(r['loadgen_returncode'] for r in report['runs']) else 0


if __name__ == '__main__':
    raise SystemExit(main())
