#!/usr/bin/env python3
"""Package existing frozen binaries in pinned Debian13; no remote operations."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
with tempfile.TemporaryDirectory(prefix='gelabber-probe-images-') as directory:
    for engine in ['current', 'mediasoup']:
        name = 'gelabber-' + engine + '-probe'
        source = ROOT / f'{engine}-probe/target/release/{name}'
        binary = Path(directory) / name
        shutil.copy2(source, binary)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        tag = f'gelabber-bench/{engine}:debian13'
        subprocess.run(['docker', 'build', '--target', engine, '-f', str(ROOT / 'docker/Dockerfile'),
                        '--build-arg', 'REVISION=' + revision, '--build-arg', 'BINARY_SHA256=' + digest,
                        '--tag', tag, directory], check=True)
        # Loader compatibility alone is insufficient: start Tokio/native C++.
        # /health can run without Redis; no external networking is allowed here.
        result = subprocess.run(['docker', 'run', '--rm', '--network', 'none', '--entrypoint', 'timeout',
            '-e', 'BENCH_TOKEN=abcdefghijklmnopqrstuvwxyz1234567890', '-e', 'BENCH_ADDR=127.0.0.1:8091',
            '-e', 'MEDIA_ADDR=127.0.0.1:8091', '-e', 'REDIS_URL=redis://127.0.0.1:1',
            tag, '3', '/usr/local/bin/' + name], capture_output=True, text=True)
        if result.returncode != 124 or 'listening on' not in result.stdout:
            raise RuntimeError(f'{engine} Debian13 startup failed: {result.stdout} {result.stderr}')
        print(subprocess.check_output(['docker', 'inspect', '--format', '{{.Id}} {{.Size}}', tag], text=True).strip())
