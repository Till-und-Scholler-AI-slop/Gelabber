"""CI/release gate and image-set contract. Registry errors fail closed."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

SERVICES = ('api', 'web', 'media')
MINIO_PIN = 'RELEASE.2025-10-15T17-29-55Z'


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def api(path):
    return json.loads(run('gh', 'api', f'repos/{os.environ["GITHUB_REPOSITORY"]}/{path}'))


def check_ci(ci, sha, repo):
    if not (ci['head_sha'] == sha and ci['head_branch'] == 'main'
            and ci['event'] == 'push' and ci['status'] == 'completed'
            and ci['conclusion'] == 'success' and ci['path'] == '.github/workflows/ci.yml'
            and ci['repository']['full_name'] == repo
            and ci['head_repository']['full_name'] == repo):
        raise ValueError('No successful trusted main CI for the exact source SHA')


def registry_digest(ref):
    result = subprocess.run(['docker', 'buildx', 'imagetools', 'inspect', ref,
                             '--format', '{{.Manifest.Digest}}'], text=True, capture_output=True)
    if result.returncode:
        # Only the registry's explicit missing-manifest response permits creation.
        if ('manifest unknown' in result.stderr.lower()
                or result.stderr.strip() == f'ERROR: {ref}: not found'):
            return None
        raise RuntimeError(f'Registry inspection failed for {ref}: {result.stderr}')
    digest = result.stdout.strip()
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
        raise ValueError(f'Invalid digest for {ref}: {digest}')
    return digest


def tag_sha(tag):
    result = subprocess.run(['git', 'rev-parse', '--verify', f'refs/tags/{tag}^{{commit}}'],
                            text=True, capture_output=True)
    return result.stdout.strip() if result.returncode == 0 else None


def promotion_allowed(sha, main, existing_tag):
    return sha == main and existing_tag in (None, sha)


def preflight(images, version):
    for item in images.values():
        existing = registry_digest(f'{item["image"]}:{version}')
        if existing is not None and existing != item['digest']:
            raise ValueError(f'Refusing to overwrite {item["image"]}:{version}: different digest')


def promote():
    repo = os.environ['GITHUB_REPOSITORY']
    sha = os.environ['SOURCE_SHA']
    ci_id = int(os.environ['CI_RUN_ID'])
    check_ci(api(f'actions/runs/{ci_id}'), sha, repo)
    version = 'v' + tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
    images = {name: json.loads(Path(f'digests/{name}.json').read_text()) for name in SERVICES}
    for name, item in images.items():
        if (item['revision'] != sha or item['image'] != f'ghcr.io/{repo.lower()}/{name}'
                or not re.fullmatch(r'sha256:[0-9a-f]{64}', item['digest'])):
            raise ValueError('Image set does not match tested source')
        if registry_digest(item['image'] + ':' + item['tag']) != item['digest']:
            raise ValueError('SHA image digest changed')
    minio = f'ghcr.io/{repo.lower()}/minio'
    minio_digest = registry_digest(f'{minio}:{MINIO_PIN}')
    if minio_digest is None:
        raise ValueError('Pinned MinIO image is missing')
    # Do not mutate the shipped version on an unchanged-version main commit.
    allowed = promotion_allowed(sha, api('git/ref/heads/main')['object']['sha'], tag_sha(version))
    if allowed:
        preflight(images, version)  # Inspect the entire set BEFORE writing any version tag.
        for item in images.values():
            run('docker', 'buildx', 'imagetools', 'create', '--prefer-index=false',
                '--tag', f'{item["image"]}:{version}', f'{item["image"]}@{item["digest"]}')
            if registry_digest(f'{item["image"]}:{version}') != item['digest']:
                raise ValueError('Promoted digest does not match candidate')
    else:
        print('Version already shipped at another SHA, or main moved: SHA set only; no promotion.')
    manifest = dict(schema=1, revision=sha, version=version, ci_run_id=ci_id,
                    publish_run_id=int(os.environ['GITHUB_RUN_ID']),
                    promoted=allowed, images=images,
                    minio=dict(image=minio, digest=minio_digest, source_pin=MINIO_PIN))
    Path('image-set.json').write_text(json.dumps(manifest, indent=2) + '\n')
    entries = {**images, 'minio': manifest['minio']}
    Path('image-set.env').write_text(''.join(
        f'GELABBER_{name.upper()}_IMAGE={item["image"]}@{item["digest"]}\n'
        for name, item in entries.items()))


def latest():
    """Refresh mutable aliases only from the latest published stable release."""
    repo = os.environ['GITHUB_REPOSITORY']
    manifest = json.loads(Path('image-set.json').read_text())
    sha, version = manifest['revision'], manifest['version']
    if (manifest.get('promoted') is not True or version != os.environ['RELEASE_TAG']
            or not re.fullmatch(r'v\d+\.\d+\.\d+', version)
            or not re.fullmatch(r'[0-9a-f]{40}', sha)):
        raise ValueError('Latest requires a promoted stable release image set')
    published = api('releases/latest')
    if (published['tag_name'] != version or published['draft']
            or published['prerelease'] or not published.get('published_at')):
        raise ValueError('Refusing to move latest to an older or unpublished release')
    # gh release create may have just created this tag on the remote.
    run('git', 'fetch', '--no-tags', 'origin', f'refs/tags/{version}:refs/tags/{version}')
    if tag_sha(version) != sha:
        raise ValueError('Published release tag does not match image-set source')
    # A manual repair may run after main advanced, but never from unrelated code.
    run('git', 'merge-base', '--is-ancestor', sha, 'HEAD')
    publish = api(f'actions/runs/{manifest["publish_run_id"]}')
    if (publish['conclusion'] != 'success' or publish['status'] != 'completed'
            or publish['event'] != 'workflow_run' or publish['head_branch'] != 'main'
            or publish['head_sha'] != sha or publish['repository']['full_name'] != repo
            or publish['path'] != '.github/workflows/publish-images.yml'):
        raise ValueError('Latest requires the successful trusted image publish run')
    check_ci(api(f'actions/runs/{manifest["ci_run_id"]}'), sha, repo)
    images = manifest['images']
    if set(images) != set(SERVICES):
        raise ValueError('Incomplete release image set')
    refs = {}
    for name in SERVICES:
        item = images[name]
        if item['revision'] != sha:
            raise ValueError('Invalid release image source')
        refs[name] = (item, version)
    minio = manifest['minio']
    if minio['source_pin'] != MINIO_PIN:
        raise ValueError('Unexpected MinIO pin')
    refs['minio'] = (minio, MINIO_PIN)
    # Validate the complete set before changing any mutable alias.
    for name, (item, tag) in refs.items():
        if (item['image'] != f'ghcr.io/{repo.lower()}/{name}'
                or not re.fullmatch(r'sha256:[0-9a-f]{64}', item['digest'])
                or registry_digest(f'{item["image"]}:{tag}') != item['digest']):
            raise ValueError('Release image digest or repository mismatch')
    for item, _ in refs.values():
        run('docker', 'buildx', 'imagetools', 'create', '--prefer-index=false',
            '--tag', f'{item["image"]}:latest', f'{item["image"]}@{item["digest"]}')
        if registry_digest(f'{item["image"]}:latest') != item['digest']:
            raise ValueError('Latest alias does not match the released digest')


def release():
    repo = os.environ['GITHUB_REPOSITORY']
    manifest = json.loads(Path('image-set.json').read_text())
    sha, version = manifest['revision'], manifest['version']
    if not manifest['promoted'] or version != os.environ['RELEASE_TAG']:
        raise ValueError('Only a fully promoted matching version can be released')
    publish = api(f'actions/runs/{manifest["publish_run_id"]}')
    if (publish['conclusion'] != 'success' or publish['status'] != 'completed'
            or publish['event'] != 'workflow_run' or publish['head_branch'] != 'main'
            or publish['repository']['full_name'] != repo
            or publish['path'] != '.github/workflows/publish-images.yml'):
        raise ValueError('Image publish run did not succeed')
    check_ci(api(f'actions/runs/{manifest["ci_run_id"]}'), sha, repo)
    if api('git/ref/heads/main')['object']['sha'] != sha or run('git', 'rev-parse', 'HEAD') != sha:
        raise ValueError('Release source must be current, tested main')
    expected = 'v' + tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
    if expected != version or tag_sha(version) not in (None, sha):
        raise ValueError('Version or existing tag has a different source SHA')
    for name in SERVICES:
        item = manifest['images'][name]
        if item['revision'] != sha or item['image'] != f'ghcr.io/{repo.lower()}/{name}':
            raise ValueError('Invalid image-set source')
        if registry_digest(f'{item["image"]}:{version}') != item['digest']:
            raise ValueError('Release version digest changed')
    # Do not edit or replace an existing release, even on a retry.
    releases = api('releases?per_page=100')
    if any(r['tag_name'] == version for r in releases):
        print('Release already exists at the verified SHA; preserving it.')
        latest()
        return
    args = ['gh', 'release', 'create', version, 'image-set.json', 'image-set.env',
            '--target', sha, '--title', f'Gelabber {version}']
    notes = Path(f'.github/release-notes/{version}.md')
    args += ['--notes-file', str(notes)] if notes.exists() else ['--generate-notes']
    run(*args)
    latest()


if __name__ == '__main__':
    {'promote': promote, 'release': release, 'latest': latest}[sys.argv[1]]()
