#!/usr/bin/env python3
"""Record an owned media process tree from the SERVER host, without fake load.
Usage: record.py --pid PID --label mediasoup-16-video-r1 --duration 90 --output FILE
Docker: add --container ID; cgroup total memory/CPU is then recorded as well.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def hardware():
    cpu = next((line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines()
                if line.startswith('model name')), None)
    memory = next((int(line.split()[1]) * 1024 for line in Path('/proc/meminfo').read_text().splitlines()
                   if line.startswith('MemTotal:')), None)
    return {'cpu': cpu, 'logical_cpus': os.cpu_count(), 'total_ram_bytes': memory,
            'kernel': os.uname().release, 'cpu_affinity': sorted(os.sched_getaffinity(0))}


def process_tree(root):
    pids = {root}
    while True:
        old = set(pids)
        for pid in old:
            # A child can be forked by a non-main thread (native workers and
            # browser launchers); inspect every task, not just the leader.
            for children in Path(f'/proc/{pid}/task').glob('*/children'):
                try:
                    pids.update(map(int, children.read_text().split()))
                except FileNotFoundError:
                    pass  # A thread may finish while this snapshot is read.
        if pids == old:
            return sorted(pids)


def process_sample(pid):
    status = Path(f'/proc/{pid}/status').read_text()
    values = {line.split(':')[0]: line.split(':', 1)[1].strip() for line in status.splitlines() if ':' in line}
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    rss = int(values.get('VmRSS', '0 kB').split()[0]) * 1024
    pss = None
    try:
        for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
            if line.startswith('Pss:'):
                pss = int(line.split()[1]) * 1024
    except PermissionError:
        pass
    return {'pid': pid, 'rss_bytes': rss, 'pss_bytes': pss, 'cpu_seconds': (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK'), 'threads': int(values['Threads'])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--label', required=True)
    parser.add_argument('--duration', type=float, default=90)
    parser.add_argument('--interval', type=float, default=1)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--container')
    args = parser.parse_args()
    if args.duration <= 0 or args.interval <= 0:
        parser.error('duration and interval must be positive')
    report = {'schema': 1, 'server_hostname': socket.gethostname(), 'server_hardware': hardware(), 'label': args.label, 'root_pid': args.pid, 'container': args.container, 'started_at': time.time(), 'samples': [], 'measurement': 'entire owned media process tree, not load generator or shared Redis'}
    cgroup = None
    if args.container:
        inspected = json.loads(subprocess.check_output(['docker', 'inspect', args.container], text=True))[0]
        if inspected['State']['Pid'] != args.pid:
            parser.error('PID does not match the owned container')
        report['image_id'] = inspected['Image']
        report['container_labels'] = inspected['Config']['Labels']
        groups = Path(f'/proc/{args.pid}/cgroup').read_text().splitlines()
        v2 = next((line.split('::', 1)[1] for line in groups if line.startswith('0::')), None)
        if v2:
            cgroup = Path('/sys/fs/cgroup') / v2.lstrip('/')
    try:
        end = time.monotonic() + args.duration
        while time.monotonic() < end:
            children = [process_sample(pid) for pid in process_tree(args.pid)]
            sample = {'at': time.time(), 'processes': children, 'rss_bytes': sum(p['rss_bytes'] for p in children), 'cpu_seconds': sum(p['cpu_seconds'] for p in children)}
            if cgroup:
                sample['cgroup_memory_current'] = int((cgroup / 'memory.current').read_text())
                sample['cgroup_cpu_stat'] = (cgroup / 'cpu.stat').read_text()
            report['samples'].append(sample)
            time.sleep(min(args.interval, max(0, end - time.monotonic())))
    except (KeyboardInterrupt, FileNotFoundError) as error:
        report['error'] = type(error).__name__
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['samples'] and 'error' not in report else 1


if __name__ == '__main__':
    raise SystemExit(main())
