"""Which Actions caches are superseded, and their deletion (cache-cleanup.yml).

The repository's caches share a 10 GB quota. rust-cache, setup-node and the
libwebrtc job save a new entry whenever their key changes and never delete
the old one; GitHub then evicts by least recent use, whatever that hits.
This script keeps, on `main` and the development line (the newest `v*`
branch), the newest entry of each kind, and deletes the older generations
and every entry of older version branches and pull requests.

A kind is a key without its hash parts: the generations of one cache differ
only in those. Keys of other families (BuildKit's type=gha layers) are left
alone, because their entries are pieces of one cache, not generations.
"""
import json
import os
import re
import subprocess
import sys

FAMILIES = ('v0-rust-', 'node-cache-', 'libwebrtc-')
HASH = re.compile(r'^[0-9a-f]{8,}$')
VERSION_BRANCH = re.compile(r'^refs/heads/v(\d+)\.(\d+)$')


def kind(key):
    """The key without its hash parts, or None for a key of another family."""
    if not key.startswith(FAMILIES):
        return None
    return '-'.join(part for part in key.split('-') if not HASH.match(part))


def development_line(branches):
    """The newest version branch among the branch names, or None."""
    versions = [(int(found.group(1)), int(found.group(2)), name) for name in branches
                if (found := VERSION_BRANCH.match(f'refs/heads/{name}'))]
    return max(versions)[2] if versions else None


def superseded(caches, development):
    """The caches to delete: (id, key, ref, size, reason) for each."""
    kept = {'refs/heads/main'}
    if development:
        kept.add(f'refs/heads/{development}')
    doomed = []
    newest = {}
    for cache in caches:
        ref, family = cache['ref'], kind(cache['key'])
        if family is None:
            continue
        if ref.startswith(('refs/pull/', 'refs/heads/gh-readonly-queue/')):
            doomed.append((cache, 'pull requests and the merge queue only read caches'))
        elif VERSION_BRANCH.match(ref) and ref not in kept:
            doomed.append((cache, f'{ref} is older than the development line'))
        elif ref in kept:
            newest.setdefault((ref, family), []).append(cache)
    for (ref, family), generations in newest.items():
        generations.sort(key=lambda cache: cache['created_at'], reverse=True)
        for cache in generations[1:]:
            doomed.append((cache, f'{generations[0]["key"]} on {ref} is newer'))
    return [(cache['id'], cache['key'], cache['ref'], cache['size_in_bytes'], reason)
            for cache, reason in doomed]


def gh(*args):
    return subprocess.check_output(('gh', 'api', *args), text=True)


def listed(path):
    """Every item of a paginated list endpoint."""
    pages = gh('--paginate', '--slurp', path)
    return json.loads(pages)


def main(argv):
    repo = os.environ['GITHUB_REPOSITORY']
    dry_run = '--dry-run' in argv
    caches = [cache for page in listed(f'repos/{repo}/actions/caches?per_page=100')
              for cache in page['actions_caches']]
    branches = [branch['name'] for page in listed(f'repos/{repo}/branches?per_page=100') for branch in page]
    development = development_line(branches)
    doomed = superseded(caches, development)
    total = sum(cache['size_in_bytes'] for cache in caches)
    freed = sum(size for _, _, _, size, _ in doomed)
    print(f'development line: {development}; {len(caches)} caches, {total / 1e9:.1f} GB')
    for cache_id, key, ref, size, reason in doomed:
        print(f'delete {key} ({ref}, {size / 1e9:.2f} GB): {reason}')
        if not dry_run:
            gh('--method', 'DELETE', f'repos/{repo}/actions/caches/{cache_id}')
    print(f'{"would free" if dry_run else "freed"} {freed / 1e9:.1f} GB')


if __name__ == '__main__':
    main(sys.argv[1:])
