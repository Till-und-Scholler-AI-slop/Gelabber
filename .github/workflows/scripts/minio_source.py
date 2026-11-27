"""The pinned MinIO and mc images and the sources they were built from (GEL-402).

Each image carries the git tree hash of its folder (deploy/compose/minio,
deploy/compose/mc) as the label org.gelabber.source-tree. publish-minio.yml
pushes a missing tag, rebuilds and replaces one whose label differs from the
folder at the pushed commit (or has none), and keeps the rest. A missed or
half-finished run is therefore finished by the next one, a manual one too.
image_set.py promote waits until the MinIO tag carries the release commit's
tree before it records the tag's digest in the image set.

Usage: minio_source.py plan                 (lines for $GITHUB_OUTPUT)
       minio_source.py publish MINIO MC     (each push, replace or keep)
"""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

LABEL = 'org.gelabber.source-tree'
FOLDERS = {'minio': 'deploy/compose/minio', 'mc': 'deploy/compose/mc'}
COMPOSE = Path('deploy/compose/compose.yaml')
ACTIONS = ('push', 'replace', 'keep')


def run(*args):
    return subprocess.run(args, capture_output=True, text=True)


def source_tree(name, rev='HEAD'):
    """The git tree hash of an image's folder at `rev`."""
    result = run('git', 'rev-parse', f'{rev}:{FOLDERS[name]}')
    if result.returncode or not re.fullmatch(r'[0-9a-f]{40}', result.stdout.strip()):
        raise RuntimeError(f'no tree for {FOLDERS[name]} at {rev}: {result.stderr.strip()}')
    return result.stdout.strip()


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
    """(digest, source tree or '') of a published tag, or None when the tag
    is missing. Any other registry error raises."""
    result = run('docker', 'buildx', 'imagetools', 'inspect', ref,
                 '--format', '{{.Manifest.Digest}} {{json .Image.Config.Labels}}')
    if result.returncode:
        if 'manifest unknown' in result.stderr.lower() or result.stderr.strip() == f'ERROR: {ref}: not found':
            return None
        raise RuntimeError(f'Registry inspection failed for {ref}: {result.stderr.strip()}')
    digest, _, labels = result.stdout.strip().partition(' ')
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
        raise RuntimeError(f'Invalid digest for {ref}: {digest}')
    return digest, (json.loads(labels or 'null') or {}).get(LABEL, '')


def action(found, tree):
    """push a missing tag, replace one built from other sources, keep the rest."""
    if found is None:
        return 'push'
    return 'keep' if found[1] == tree else 'replace'


def plan(compose):
    """Each image's reference, source tree and action."""
    out = {}
    for name in FOLDERS:
        ref, tree = pinned(name, compose), source_tree(name)
        out[name] = (ref, tree, action(published(ref), tree))
    return out


def publish(actions, compose):
    """Push or replace what the plan said, then check the label arrived."""
    unknown = {name: act for name, act in actions.items() if act not in ACTIONS}
    if unknown:
        raise ValueError(f'unknown actions {unknown}; nothing pushed')
    for name, act in actions.items():
        ref = pinned(name, compose)
        if act == 'keep':
            print(f'{ref} is built from this commit\'s {FOLDERS[name]}; keeping it.')
            continue
        print(f'{"Pushing" if act == "push" else "Replacing"} {ref}.')
        local = 'gelabber/' + ref.rsplit('/', 1)[1]
        for args in (('docker', 'tag', local, ref), ('docker', 'push', ref)):
            result = run(*args)
            if result.returncode:
                raise RuntimeError(f'{" ".join(args)} failed: {result.stderr.strip()}')
        found = published(ref)
        if found is None or found[1] != source_tree(name):
            raise RuntimeError(f'{ref} does not carry the tree of {FOLDERS[name]} after the push')


def wait_for(ref, tree, bound=15 * 60, every=30, clock=time.monotonic, sleep=time.sleep):
    """The digest of `ref` once it is built from `tree`; publish-minio.yml may
    still be building it. Raises after `bound` seconds."""
    deadline = clock() + bound
    while True:
        found = published(ref)
        if found is not None and found[1] == tree:
            return found[0]
        if clock() >= deadline:
            raise ValueError(f'{ref} is not built from tree {tree} after {bound // 60} minutes: '
                             'see publish-minio.yml for this commit, then re-run this job')
        print(f'{ref} is not built from tree {tree} yet; waiting for publish-minio.yml.', file=sys.stderr)
        sleep(every)


def main(argv):
    compose = COMPOSE.read_text()
    if argv[:1] == ['plan']:
        planned = plan(compose)
        for name, (_, tree, act) in planned.items():
            print(f'{name}={act}')
            print(f'{name}-tree={tree}')
        print(f'build={"true" if any(act != "keep" for *_, act in planned.values()) else "false"}')
    elif argv[:1] == ['publish'] and len(argv) == 3:
        publish(dict(zip(FOLDERS, argv[1:])), compose)
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
