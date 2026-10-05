#!/usr/bin/env python3
"""Freeze a private local generator image and verify its executed Opus fixtures.

Default is a read-only plan. --execute creates only uniquely labelled local
images/containers, without networking, packages, ports or production changes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid

ROOT = Path(__file__).resolve().parent
OPUS_SHA = 'ce07b3578b14e1d25ed603670f2336cd2b32b7f24c2c5b9aab8bbeb0f410b8d6'
NODE_SHA = '8a22a371fd85aecf5411636574309f6380fbc42694aaf0651a089a8ef9c44e52'
BASE = 'sha256:a1087f71b2dbd30bd90b9f7dbd7dc42b2781c21b3c5873b14fc81ec9d158cc1b'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args, **options):
    try:
        return subprocess.run(args, check=True, capture_output=True, text=True, timeout=120, **options).stdout.strip()
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f'command exited {error.returncode}: {args!r}; stdout={error.stdout[-8192:]}; stderr={error.stderr[-8192:]}') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-image', default=BASE)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--node', type=Path, required=True)
    parser.add_argument('--native-binary', type=Path, required=True)
    parser.add_argument('--video-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--keep-image', action='store_true')
    args = parser.parse_args()
    if args.base_image != BASE:
        parser.error('this control requires the independently recorded Debian13 generator base image ID')
    inputs = {'libopus.so.0': args.library.resolve(), 'node': args.node.resolve(),
              'native-peer': args.native_binary.resolve(), 'video.rtpbin': args.video_archive.resolve(),
              'opus-fixture.py': ROOT / 'opus-fixture.py', 'prepare-native-runtime.py': Path(__file__).resolve()}
    for name in ('Cargo.toml', 'Cargo.lock', 'src/full_peer.rs', 'src/archive.rs', 'src/audio.rs', 'src/clock.rs', 'src/unique_json.rs'):
        inputs['rust-' + name.replace('/', '-')] = ROOT / 'rtp-source' / name
    hashes = {name: digest(path) for name, path in inputs.items()}
    if hashes['libopus.so.0'] != OPUS_SHA or hashes['node'] != NODE_SHA:
        parser.error('actual libopus1.6.1 or Node26.8.2 binary differs from frozen pin')
    run_id = 'gelabber-native-runtime-' + uuid.uuid4().hex[:12]
    image, base_tag = run_id + ':owned', run_id + ':base'
    dockerfile = '\n'.join([f'FROM {base_tag}', 'USER root', 'COPY inputs/libopus.so.0 /fixture-lib/libopus.so.0',
        'COPY inputs/node /fixture-bin/node', 'COPY inputs/native-peer /fixture-bin/native-peer',
        'ENV LD_LIBRARY_PATH=/fixture-lib', 'ENTRYPOINT []', 'CMD ["sleep","infinity"]', ''])
    report = {'schema': 1, 'scope': 'local offline generator/decoder controls; no SFU, WAN or latency acceptance',
        'execute': args.execute, 'run_id': run_id, 'image': image, 'base_image_id': BASE,
        'input_hashes': hashes, 'dockerfile': dockerfile, 'limits': {'cpu': 1, 'memory': '512m', 'pids': 256},
        'network': 'none', 'comparison_available': False, 'pcm_latency_calibrated': False,
        'git_revision': run(['git', '-C', str(ROOT), 'rev-parse', 'HEAD']),
        'git_dirty': bool(run(['git', '-C', str(ROOT), 'status', '--porcelain']))}
    if not args.execute:
        print(json.dumps(report, indent=2)); return 0
    args.output.mkdir(parents=True, exist_ok=False)
    folder = args.output.resolve(); frozen = folder / 'inputs'; frozen.mkdir()
    for name, path in inputs.items():
        shutil.copy2(path, frozen / name)
        if digest(frozen / name) != hashes[name]:
            raise ValueError('input changed while freezing: ' + name)
    (folder / 'Dockerfile').write_text(dockerfile)
    controls = folder / 'controls'; controls.mkdir(mode=0o700)
    created_base = False; created_image = False
    try:
        actual = json.loads(run(['docker', 'image', 'inspect', BASE]))[0]
        if actual['Id'] != BASE:
            raise ValueError('actual base image ID differs')
        report['base_image_labels'] = actual['Config'].get('Labels')
        run(['docker', 'tag', BASE, base_tag]); created_base = True
        run(['docker', 'build', '--network=none', '--label', 'gelabber.bench.native-runtime=' + run_id,
            '--label', 'gelabber.bench.libopus.sha256=' + OPUS_SHA, '--label', 'gelabber.bench.node.sha256=' + NODE_SHA,
            '--label', 'gelabber.bench.native.sha256=' + hashes['native-peer'], '--tag', image, str(folder)])
        created_image = True
        inspected = json.loads(run(['docker', 'image', 'inspect', image]))[0]
        report['image_id'] = inspected['Id']; report['image_labels'] = inspected['Config'].get('Labels')
        common = ['docker', 'run', '--rm', '--name', run_id, '--label', 'gelabber.bench.native-runtime=' + run_id,
            '--network=none', '--read-only', '--tmpfs', '/tmp:rw,nosuid,nodev,size=64m', '--cpus=1', '--memory=512m',
            '--pids-limit=256', '--user', f'{os.getuid()}:{os.getgid()}',
            '--mount', 'type=bind,src=' + str(frozen) + ',dst=/inputs,readonly',
            '--mount', 'type=bind,src=' + str(controls) + ',dst=/controls', '--workdir=/controls', image]
        report['executed_node'] = run(common + ['/fixture-bin/node', '--version'])
        if report['executed_node'] != 'v26.8.2':
            raise ValueError('executed Node version differs')
        report['executed_native_ldd'] = run(common + ['ldd', '/fixture-bin/native-peer'])
        report['executed_library_ldd'] = run(common + ['ldd', '/fixture-lib/libopus.so.0'])
        report['executed_glibc'] = run(common + ['getconf', 'GNU_LIBC_VERSION'])
        for kind in ('mic', 'source'):
            run(common + ['python3', '/inputs/opus-fixture.py', '--library', '/fixture-lib/libopus.so.0',
                'build', kind, '/controls/' + kind + '.opusbin', '--output', '/controls/' + kind + '.json', '--pcm', '/controls/' + kind + '.f32'])
            inspected = json.loads(run(common + ['python3', '/inputs/opus-fixture.py', '--library', '/fixture-lib/libopus.so.0',
                'inspect', '/controls/' + kind + '.opusbin']))
            (controls / (kind + '-inspect.json')).write_text(json.dumps(inspected, indent=2))
        report['native_import'] = json.loads(run(common + ['/fixture-bin/native-peer', '--inspect', '/inputs/video.rtpbin',
            '/controls/mic.opusbin', '/controls/source.opusbin']))
        if report['native_import']['decoder']['sha256'] != OPUS_SHA:
            raise ValueError('actual native mapped decoder differs')
        report['offline_controls_valid'] = True
        report['retained_image'] = image if args.keep_image else None
    except Exception as error:
        report['error'] = str(error); report['offline_controls_valid'] = False
    finally:
        cleanup_errors = []
        # Exact UUID name AND label, never a broad filter or a foreign fixture.
        owned = ''
        try: owned = run(['docker', 'ps', '-aq', '--filter', 'name=^/' + run_id + '$', '--filter', 'label=gelabber.bench.native-runtime=' + run_id])
        except Exception as error: cleanup_errors.append(str(error))
        if owned:
            try: run(['docker', 'rm', '--force', owned])
            except Exception as error: cleanup_errors.append(str(error))
        if created_image and (not args.keep_image or not report.get('offline_controls_valid')):
            try: run(['docker', 'image', 'rm', image])
            except Exception as error: cleanup_errors.append(str(error))
        if created_base:
            try: run(['docker', 'image', 'rm', base_tag])
            except Exception as error: cleanup_errors.append(str(error))
        report['cleanup_errors'] = cleanup_errors
        report['cleanup_valid'] = not cleanup_errors
        (folder / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps({'report': str(folder / 'report.json'), 'offline_controls_valid': report.get('offline_controls_valid'),
        'cleanup_valid': report['cleanup_valid'], 'retained_image': report.get('retained_image')}, indent=2))
    return 0 if report.get('offline_controls_valid') and report['cleanup_valid'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
