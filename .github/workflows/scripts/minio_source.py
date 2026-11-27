"""The pinned MinIO and mc images and the sources they were built from (GEL-402).

Each image carries its sources as the label org.gelabber.source-tree: the git
tree hash of its folder (deploy/compose/minio, deploy/compose/mc) and the blob
hash of minio-images.yml, which builds it. publish-minio.yml pushes a missing
tag, rebuilds and replaces one whose label differs from those at main's head
(or has none), and keeps the rest; a missed or half-finished run is finished
by the next one. A run of an older commit, a re-run for example, publishes
nothing. image_set.py promote waits until the MinIO tag carries the release
commit's sources before it records the tag's digest in the image set.

Usage: minio_source.py source NAME [REV]    (the label value at REV)
       minio_source.py plan                 (lines for $GITHUB_OUTPUT)
       minio_source.py publish MINIO MC     (each push, replace or keep)
"""
import json
from pathlib import Path
import os
import re
import subprocess
import sys
import time

LABEL = 'org.gelabber.source-tree'
FOLDERS = {'minio': 'deploy/compose/minio', 'mc': 'deploy/compose/mc'}
BUILD = '.github/workflows/minio-images.yml'
COMPOSE = Path('deploy/compose/compose.yaml')
ACTIONS = ('push', 'replace', 'keep')
INDEXES = ('application/vnd.oci.image.index.v1+json',
           'application/vnd.docker.distribution.manifest.list.v2+json')
MOVED_ON = 'main has moved on to other MinIO sources: run publish-minio.yml on main'


class MultiPlatformIndex(RuntimeError):
    """A tag that is an index: waiting does not change that."""


def run(*args):
    return subprocess.run(args, capture_output=True, text=True)


def source(name, rev='HEAD'):
    """An image's sources at `rev`: its folder's tree and the build workflow's blob."""
    result = run('git', 'rev-parse', f'{rev}:{FOLDERS[name]}', f'{rev}:{BUILD}')
    hashes = result.stdout.split()
    if result.returncode or len(hashes) != 2 or not all(re.fullmatch(r'[0-9a-f]{40}', h) for h in hashes):
        raise RuntimeError(f'no sources for {name} at {rev}: {result.stderr.strip()}')
    return '+'.join(hashes)


def main_head():
    """main's head on the remote, fetched for its trees."""
    # --depth only in a shallow clone: in promote's full one it would cut history.
    shallow = run('git', 'rev-parse', '--is-shallow-repository').stdout.strip() == 'true'
    result = run('git', 'fetch', '--quiet', *(['--depth=1'] if shallow else []), 'origin', 'main')
    if result.returncode:
        raise RuntimeError(f'cannot fetch main: {result.stderr.strip()}')
    return run('git', 'rev-parse', 'FETCH_HEAD').stdout.strip()


def require_current(rev='HEAD'):
    """Refuse to publish sources that main no longer has."""
    head = main_head()
    for name in FOLDERS:
        if source(name, rev) != source(name, head):
            raise ValueError(f'{MOVED_ON} ({name}: {rev} is not main\'s {head}); nothing pushed')


def pinned(name, compose):
    """The image reference compose.yaml starts by default, with its tag."""
    found = re.search(rf'\$\{{GELABBER_{name.upper()}_IMAGE:-([^}}\s]+/{name}:[^}}\s]+)\}}', compose)
    if not found:
        raise ValueError(f'compose.yaml pins no {name} image')
    repo = os.environ.get('GITHUB_REPOSITORY', '').lower()
    if repo and not found.group(1).startswith(f'ghcr.io/{repo}/'):
        raise ValueError(f'compose.yaml pins {found.group(1)}, not an image of {repo}')
    return found.group(1)


def published(ref):
    """(digest, sources or '') of a published tag, or None when the tag is
    missing. Any other registry error raises, and so does a multi-platform
    index, whose labels this cannot read."""
    result = run('docker', 'buildx', 'imagetools', 'inspect', ref,
                 '--format', '{{.Manifest.Digest}} {{.Manifest.MediaType}} {{json .Image.Config.Labels}}')
    if result.returncode:
        if 'manifest unknown' in result.stderr.lower() or result.stderr.strip() == f'ERROR: {ref}: not found':
            return None
        raise RuntimeError(f'Registry inspection failed for {ref}: {result.stderr.strip()}')
    digest, media_type, labels = (result.stdout.strip().split(' ', 2) + ['', ''])[:3]
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
        raise RuntimeError(f'Invalid digest for {ref}: {digest}')
    if media_type in INDEXES:
        raise MultiPlatformIndex(f'{ref} is a multi-platform index ({media_type}); its labels are per platform, '
                           'and publish-minio.yml pushes single images')
    return digest, (json.loads(labels or 'null') or {}).get(LABEL, '')


def action(found, sources):
    """push a missing tag, replace one built from other sources, keep the rest."""
    if found is None:
        return 'push'
    return 'keep' if found[1] == sources else 'replace'


def plan(compose):
    """Each image's action, from main's head only."""
    require_current()
    return {name: action(published(pinned(name, compose)), source(name)) for name in FOLDERS}


def publish(actions, compose):
    """Push or replace what the plan said, then check the label arrived."""
    unknown = {name: act for name, act in actions.items() if act not in ACTIONS}
    if unknown:
        raise ValueError(f'unknown actions {unknown}; nothing pushed')
    require_current()
    for name, act in actions.items():
        ref = pinned(name, compose)
        if act == 'keep':
            print(f'{ref} is built from main\'s sources; keeping it.')
            continue
        print(f'{"Pushing" if act == "push" else "Replacing"} {ref}.')
        local = 'gelabber/' + ref.rsplit('/', 1)[1]
        for args in (('docker', 'tag', local, ref), ('docker', 'push', ref)):
            result = run(*args)
            if result.returncode:
                raise RuntimeError(f'{" ".join(args)} failed: {result.stderr.strip()}')
        found = published(ref)
        if found is None or found[1] != source(name):
            raise RuntimeError(f'{ref} does not carry this commit\'s sources after the push')


def wait_for(ref, sources, bound=15 * 60, every=30, clock=time.monotonic, sleep=time.sleep):
    """The digest of `ref` once it is built from `sources`; publish-minio.yml
    may still be building it. Registry errors are retried; raises after
    `bound` seconds."""
    deadline = clock() + bound
    while True:
        try:
            found = published(ref)
        except MultiPlatformIndex:
            raise
        except RuntimeError as error:
            found, problem = None, str(error)
        else:
            problem = 'not built from them yet'
        if found is not None and found[1] == sources:
            return found[0]
        if clock() >= deadline:
            raise ValueError(f'{ref} is not built from {sources} after {bound // 60} minutes ({problem}): '
                             'run publish-minio.yml on main, then re-run this job')
        print(f'{ref}: {problem}; waiting for publish-minio.yml.', file=sys.stderr)
        sleep(every)


def main(argv):
    if argv[:1] == ['source'] and len(argv) in (2, 3):
        print(source(*argv[1:]))
        return
    compose = COMPOSE.read_text()
    if argv[:1] == ['plan']:
        planned = plan(compose)
        for name, act in planned.items():
            print(f'{name}={act}')
        print(f'build={"true" if any(act != "keep" for act in planned.values()) else "false"}')
    elif argv[:1] == ['publish'] and len(argv) == 3:
        publish(dict(zip(FOLDERS, argv[1:])), compose)
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
