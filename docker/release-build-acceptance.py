"""Non-publishing Docker acceptance on an isolated runner; reports real GHA cache transfers."""
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = sys.argv[1]
assert PACKAGE in ('api', 'media')
BASELINE = sys.argv[2]
REPORT = ROOT / 'build-acceptance' / PACKAGE
REPORT.mkdir(parents=True, exist_ok=True)
RUN_ID = os.environ['GITHUB_RUN_ID']
REVISION = os.environ['GITHUB_SHA']
PREFIX = f'acceptance-{PACKAGE}-{RUN_ID}'
RESULTS = []
CONTAINERS = []
VERSION = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
CREATED = subprocess.check_output(['git', 'show', '-s', '--format=%cI', 'HEAD'], cwd=ROOT, text=True).strip()
SOURCE = f'https://github.com/{os.environ["GITHUB_REPOSITORY"]}'


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def command(*args):
    subprocess.run(args, check=True)


def wait_ready(url):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=3) as response:
                if response.status == 200:
                    return response.read().decode()
        except Exception:
            pass
        time.sleep(1)
    raise RuntimeError(f'Readiness failed: {url}')


def container(name, image, *args):
    full_name = f'{PREFIX}-{name}'
    command('docker', 'run', '-d', '--name', full_name, '--network', PREFIX, *args, image)
    CONTAINERS.append(full_name)
    return full_name


def services():
    command('docker', 'network', 'create', PREFIX)
    container('redis', 'redis:8.10.1-alpine', '--network-alias', 'redis')
    if PACKAGE == 'api':
        postgres = container('postgres', 'postgres:18.6-alpine3.24', '--network-alias', 'postgres',
                             '-e', 'POSTGRES_USER=gelabber', '-e', 'POSTGRES_PASSWORD=acceptance-only',
                             '-e', 'POSTGRES_DB=gelabber')
        deadline = time.monotonic() + 90
        while subprocess.run(['docker', 'exec', postgres, 'pg_isready', '-h', '127.0.0.1', '-U', 'gelabber', '-d', 'gelabber'],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
            if time.monotonic() > deadline:
                raise RuntimeError('Postgres failed readiness')
            time.sleep(1)
        image = f'ghcr.io/{os.environ["GITHUB_REPOSITORY"].lower()}/minio:RELEASE.2025-10-15T17-29-55Z'
        # MinIO uses an explicit command after its image name.
        name = f'{PREFIX}-minio'
        command('docker', 'run', '-d', '--name', name, '--network', PREFIX,
                '--network-alias', 'minio', '-p', '127.0.0.1:19000:9000',
                '-e', 'MINIO_ROOT_USER=acceptance-only', '-e', 'MINIO_ROOT_PASSWORD=acceptance-only-password',
                image, 'server', '/data')
        CONTAINERS.append(name)
        wait_ready('http://127.0.0.1:19000/minio/health/ready')


def smoke(image, label):
    port = '8080' if PACKAGE == 'api' else '8081'
    args = ['-p', f'127.0.0.1:18080:{port}', '-e', 'REDIS_URL=redis://redis:6379']
    if PACKAGE == 'api':
        args += ['-e', 'DATABASE_URL=postgres://gelabber:acceptance-only@postgres:5432/gelabber',
                 '-e', 'MINIO_ENDPOINT=http://minio:9000', '-e', 'MINIO_ROOT_USER=acceptance-only',
                 '-e', 'MINIO_ROOT_PASSWORD=acceptance-only-password', '-e', 'MINIO_BUCKET=acceptance-only']
    name = container(label, image, *args)
    try:
        body = wait_ready('http://127.0.0.1:18080/ready')
        assert json.loads(body)['status'] == 'ready', body
        (REPORT / f'{label}.ready.json').write_text(body)
        inspect = json.loads(run('docker', 'inspect', name))[0]
        assert inspect['Config']['User'] == 'nobody'
        assert inspect['Config']['Cmd'] == [f'gelabber-{PACKAGE}']
    finally:
        (REPORT / f'{label}.container.log').write_text(run('docker', 'logs', name))
        command('docker', 'container', 'stop', name)
        command('docker', 'container', 'rm', name)
        CONTAINERS.remove(name)


def cache_evidence(text, stage):
    match = re.search(r'^#(\d+) \[' + stage + r' [^\]]+\] RUN cargo build', text, re.M)
    assert match, f'{stage} build step missing'
    vertex = match[1]
    if re.search(r'^#' + vertex + r' CACHED$', text, re.M):
        return dict(cached=True, source='cached-vertex', vertex=vertex)
    # A slow content-hash cache match can restore the compiled layer without a
    # CACHED line. Require a completed layer download and no command execution.
    # Cargo always writes progress/Finished, including a no-op compilation.
    restored = bool(re.search(r'^#' + vertex + r' extracting sha256:', text, re.M))
    completed = bool(re.search(r'^#' + vertex + r' DONE', text, re.M))
    executed = bool(re.search(r'^#' + vertex + r' \d+(?:\.\d+)? ', text, re.M))
    return dict(cached=restored and completed and not executed,
                source='restored-layer' if restored and completed and not executed else 'executed',
                vertex=vertex)


def build(context, variant, scenario, version, dependencies_cached=None, build_cached=None):
    builder = f'{PREFIX}-{variant}-{scenario}'
    image = f'gelabber-acceptance/{PACKAGE}:{variant}-{scenario}'
    scope = f'{PREFIX}-{variant}'
    log_path = REPORT / f'{variant}-{scenario}.build.log'
    command('docker', 'buildx', 'create', '--name', builder, '--driver', 'docker-container')
    started = time.monotonic()
    try:
        with log_path.open('w') as log:
            args = ['docker', 'buildx', 'build', '--builder', builder, '--progress', 'plain',
                    '--file', str(context / PACKAGE / 'Dockerfile'), '--load', '--provenance=false',
                    '--tag', image, '--cache-from', f'type=gha,version=2,scope={scope}',
                    '--cache-to', f'type=gha,version=2,mode=max,scope={scope}',
                    '--build-arg', f'OCI_REVISION={REVISION}', '--build-arg', f'OCI_VERSION=v{version}',
                    '--build-arg', f'OCI_CREATED={CREATED}', '--build-arg', f'OCI_SOURCE={SOURCE}', str(context)]
            subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, check=True)
        elapsed = time.monotonic() - started
    finally:
        command('docker', 'buildx', 'rm', builder)
    text = log_path.read_text()
    evidence = {'build': cache_evidence(text, 'build')}
    if dependencies_cached is not None:
        evidence['dependencies'] = cache_evidence(text, 'dependencies')
        assert evidence['dependencies']['cached'] == dependencies_cached, f'Dependency cache: {scenario}'
    if build_cached is not None:
        assert evidence['build']['cached'] == build_cached, f'Final build cache: {scenario}'
    compiled = sorted(set(re.findall(r'Compiling ([\w-]+) v', text)))
    external = sorted(set(compiled) - {'gelabber-api', 'gelabber-media', 'gelabber-shared'})
    if dependencies_cached:
        assert not external, f'Cached dependency stage recompiled external packages: {external}'
    if build_cached is False:
        assert f'Compiling gelabber-{PACKAGE} v{version}' in text, 'Real release package was not compiled'
    labels = json.loads(run('docker', 'image', 'inspect', image))[0]['Config']['Labels']
    for key, value in {'revision': REVISION, 'version': f'v{version}', 'created': CREATED, 'source': SOURCE}.items():
        assert labels[f'org.opencontainers.image.{key}'] == value
    RESULTS.append(dict(variant=variant, scenario=scenario, seconds=round(elapsed, 3), version=version,
                        labels=labels, dependencies_cached=dependencies_cached, build_cached=build_cached,
                        cache_evidence=evidence, compiled_packages=compiled))
    (REPORT / 'results.json').write_text(json.dumps(RESULTS, indent=2))
    print(f'{PACKAGE} {variant} {scenario}: {elapsed:.1f}s', flush=True)
    return image


def append_comment(context, member, scenario):
    path = context / member / 'src/lib.rs'
    path.write_text(path.read_text() + f'\n// acceptance change: {scenario}\n')


try:
    services()
    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp)
        archive = temp / 'source.tar'
        command('git', 'archive', '--format=tar', '--output', str(archive), 'HEAD')
        pristine = temp / 'pristine'
        pristine.mkdir()
        command('tar', '-xf', str(archive), '-C', str(pristine))
        baseline_file = run('git', 'show', f'{BASELINE}:{PACKAGE}/Dockerfile') + '\n'
        def context(variant, scenario):
            dest = temp / f'{variant}-{scenario}'
            shutil.copytree(pristine, dest)
            if variant == 'baseline':
                (dest / PACKAGE / 'Dockerfile').write_text(baseline_file)
            return dest
        for variant in ('baseline', 'optimized'):
            image = build(context(variant, 'cold'), variant, 'cold', VERSION,
                          dependencies_cached=False if variant == 'optimized' else None, build_cached=False)
            smoke(image, f'{variant}-cold')
        # Alternate order to reduce time/order bias. Each build gets a new builder;
        # only the external GHA layer cache persists between comparable runs.
        for iteration in range(1, 4):
            variants = ('optimized', 'baseline') if iteration % 2 else ('baseline', 'optimized')
            for variant in variants:
                scenario = f'warm-{iteration}'
                dest = context(variant, scenario)
                append_comment(dest, PACKAGE, scenario)
                build(dest, variant, scenario, VERSION,
                      dependencies_cached=True if variant == 'optimized' else None, build_cached=False)
        for scenario in ('other-source', 'shared-source', 'version-only', 'dependency-features', 'toolchain-input'):
            dest = context('optimized', scenario)
            version = VERSION
            if scenario == 'other-source':
                # Match the last exported own-source state before changing only
                # the unrelated service, so this verifies the final build hit.
                append_comment(dest, PACKAGE, 'warm-3')
                append_comment(dest, 'media' if PACKAGE == 'api' else 'api', scenario)
            elif scenario == 'shared-source':
                append_comment(dest, 'shared', scenario)
            elif scenario == 'version-only':
                major, minor, patch = VERSION.split('.')
                version = f'{major}.{minor}.{int(patch)+1}'
                path = dest / 'Cargo.toml'
                path.write_text(path.read_text().replace(f'version = "{VERSION}"', f'version = "{version}"', 1))
                path = dest / 'Cargo.lock'
                text = path.read_text()
                for member in ('api', 'media', 'shared'):
                    text = text.replace(f'name = "gelabber-{member}"\nversion = "{VERSION}"',
                                        f'name = "gelabber-{member}"\nversion = "{version}"')
                path.write_text(text)
            elif scenario == 'dependency-features':
                path = dest / PACKAGE / 'Cargo.toml'
                text = path.read_text()
                updated = text.replace('"signal", "sync", "time"', '"signal", "sync", "time", "io-util"', 1)
                assert updated != text
                path.write_text(updated)
            elif scenario == 'toolchain-input':
                path = dest / 'rust-toolchain.toml'
                path.write_text(path.read_text() + '# cache invalidation check; compiler pin unchanged\n')
            image = build(dest, 'optimized', scenario, version,
                          dependencies_cached=scenario not in ('dependency-features', 'toolchain-input'),
                          build_cached=scenario == 'other-source')
            if scenario in ('shared-source', 'version-only'):
                smoke(image, scenario)
        medians = {variant: statistics.median(r['seconds'] for r in RESULTS
                   if r['variant'] == variant and r['scenario'].startswith('warm-'))
                   for variant in ('baseline', 'optimized')}
        improvement = 100 * (1 - medians['optimized'] / medians['baseline'])
        summary = dict(package=PACKAGE, baseline=BASELINE, revision=REVISION,
                       median_seconds=medians, improvement_percent=round(improvement, 2),
                       passed=improvement >= 20)
        (REPORT / 'summary.json').write_text(json.dumps(summary, indent=2))
        print(json.dumps(summary), flush=True)
        assert improvement >= 20, f'Warm-build improvement below target: {improvement:.2f}%'
finally:
    for name in list(reversed(CONTAINERS)):
        try:
            (REPORT / f'{name}.container.log').write_text(run('docker', 'logs', name))
            command('docker', 'container', 'stop', name)
            command('docker', 'container', 'rm', name)
        except Exception as error:
            print(f'Cleanup failed for {name}: {type(error).__name__}', flush=True)
    subprocess.run(['docker', 'network', 'rm', PREFIX], check=False)
