"""Which CI areas a pull request touches.

ci.yml pipes the pull request's changed files into this script and starts
only the jobs of the areas it prints. `--all` (release pull requests, manual
runs, pushes without a known starting point) starts every area. The prefixes
are the one list of what belongs to an area, so the deployment tests ask this
script too.
"""
import json
import sys

CI = '.github/workflows/ci.yml'
AREAS = {
    # previous_release.sh picks and checks the upgrade test's release in the rust job.
    'backend': ('backend/', 'rust-toolchain.toml', '.dockerignore', CI,
                '.github/workflows/scripts/previous_release.sh'),
    # The emoji data is generated into the backend, the web client and the
    # desktop app, and checked in the web job; landingImage.test.mjs reads the
    # drawn preview the landing page must not show; ci-fault-loop.checks.mjs
    # runs a step of e2e-extended.yml.
    'web': ('web/', 'backend/shared/data/', 'desktop/ui/assets/emoji/',
            'docs/design/v0.10/landing-preview-en.png', CI, '.github/workflows/e2e-extended.yml'),
    'desktop': ('desktop/', 'rust-toolchain.toml',
                # theme.rs checks its tokens against the web's (#561).
                'web/src/theme/tokens.fixture.json',
                # The packaging tests skip or fail through it (GEL-375), and
                # read the workflows with its neighbours (GEL-431).
                'deploy/compose/tests/prerequisites.py', 'deploy/compose/tests/workflow_data.py',
                'deploy/compose/tests/requirements.txt',
                # The workflows that build, store and release the desktop app.
                CI, '.github/workflows/desktop-native.yml', '.github/workflows/desktop-release.yml',
                '.github/workflows/libwebrtc-store.yml', '.github/workflows/scripts/apt-install.sh'),
    # The deployment tests also read the backend's build, both versions, the
    # web image's start script, the updater key and the workflows, which
    # zizmor and actionlint check here. They test the Claude Code hooks too,
    # so .claude/ is here.
    'deploy': ('deploy/', 'backend/', 'desktop/Cargo.toml', 'rust-toolchain.toml', '.dockerignore',
               '.claude/', '.github/', 'web/docker-entrypoint.d/50-gelabber-public-pages.sh',
               'desktop/packaging/updater.pub'),
    'minio': ('deploy/compose/minio/', 'deploy/compose/mc/', 'deploy/compose/minio-init.sh',
              'deploy/compose/compose.yaml',
              CI, '.github/workflows/minio-images.yml', '.github/workflows/publish-minio.yml'),
}
# What the minio job builds MinIO and mc from; otherwise it checks the
# published pinned images (minio-images.yml).
MINIO_BUILD = ('deploy/compose/minio/', 'deploy/compose/mc/', '.github/workflows/minio-images.yml')
# Read by no check: a change here alone starts nothing.
NO_AREA = ('.github/ISSUE_TEMPLATE/', '.github/release-notes/')
# The app images a pull request builds (without pushing). Each builds when
# one of its inputs changes: its Dockerfile, what that copies from the
# context (test_changes.py reads the COPY lines), the context's
# .dockerignore, the job that builds it, and the checks run on the image.
IMAGES = (
    {'name': 'api', 'context': '.', 'file': 'backend/api/Dockerfile',
     'inputs': ('.dockerignore', CI, 'rust-toolchain.toml',
                'backend/Cargo.toml', 'backend/Cargo.lock', 'backend/api/', 'backend/shared/',
                'backend/media/Cargo.toml', 'backend/build/debian-snapshot.sources',
                'backend/build/rust-build-ca.sh', 'backend/build/rust-deps-recipe.sh')},
    {'name': 'media', 'context': '.', 'file': 'backend/media/Dockerfile',
     'inputs': ('.dockerignore', CI, 'rust-toolchain.toml',
                'backend/Cargo.toml', 'backend/Cargo.lock', 'backend/media/', 'backend/shared/',
                'backend/api/Cargo.toml', 'backend/build/debian-snapshot.sources',
                'backend/build/rust-build-ca.sh', 'backend/build/rust-deps-recipe.sh',
                'backend/build/mediasoup-build-tools.py', 'backend/build/mediasoup-build-tools.lock.json',
                'backend/build/mediasoup-debian-build-tools.lock.tsv',
                'backend/build/install-mediasoup-debian-build-tools.sh',
                'backend/build/check-media-image.py')},
    {'name': 'web', 'context': 'web', 'file': 'web/Dockerfile',
     'inputs': ('web/', CI)},
)

def touches(path, prefixes):
    return any(path == prefix or (prefix.endswith('/') and path.startswith(prefix))
               for prefix in prefixes)


def areas(paths):
    """The set of areas the changed paths touch."""
    paths = [path for path in paths if path and not touches(path, NO_AREA)]
    return {area for area, prefixes in AREAS.items()
            if any(touches(path, prefixes) for path in paths)}


def images(paths):
    """The names of the images whose inputs the changed paths touch."""
    return {image['name'] for image in IMAGES
            if any(touches(path, image['inputs']) for path in paths if path)}


def outputs(touched, built, minio_build=False):
    """`name=value` lines for $GITHUB_OUTPUT; `images` is the build matrix."""
    lines = [f'{area}={"true" if area in touched else "false"}' for area in AREAS]
    lines.append(f'minio-build={"true" if minio_build else "false"}')
    matrix = [{key: image[key] for key in ('name', 'context', 'file')}
              for image in IMAGES if image['name'] in built]
    lines.append('images=' + json.dumps(matrix, separators=(',', ':')))
    return lines


def main(argv):
    if '--all' in argv:
        # Release candidates build MinIO from source too: operators fall back
        # to that build, and nobody waits on it beside the desktop jobs.
        touched, built, minio_build = set(AREAS), {image['name'] for image in IMAGES}, True
    else:
        paths = sys.stdin.read().splitlines()
        touched, built = areas(paths), images(paths)
        minio_build = any(touches(path, MINIO_BUILD) for path in paths if path)
    print('\n'.join(outputs(touched, built, minio_build)))
    print('areas: ' + (', '.join(sorted(touched)) or 'none')
          + '; images: ' + (', '.join(sorted(built)) or 'none'), file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
