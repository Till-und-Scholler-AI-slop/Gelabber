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
    'backend': ('backend/', 'rust-toolchain.toml', '.dockerignore', CI),
    # The emoji data is generated into the backend, the web client and the
    # desktop app, and checked in the web job; landingImage.test.mjs reads the
    # drawn preview the landing page must not show; ci-fault-loop.checks.mjs
    # runs a step of e2e-extended.yml.
    'web': ('web/', 'backend/shared/data/', 'desktop/ui/assets/emoji/',
            'docs/design/v0.10/landing-preview-en.png', CI, '.github/workflows/e2e-extended.yml'),
    'desktop': ('desktop/', 'rust-toolchain.toml',
                # theme.rs checks its tokens against the web's (#561).
                'web/src/theme/tokens.fixture.json',
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
              'deploy/compose/compose.yaml', 'backend/Cargo.toml',
              CI, '.github/workflows/minio-images.yml', '.github/workflows/publish-minio.yml'),
}
# Read by no check: a change here alone starts nothing.
NO_AREA = ('.github/ISSUE_TEMPLATE/', '.github/release-notes/')
# The app images a pull request builds (without pushing), by area.
IMAGES = {
    'backend': ({'name': 'api', 'context': '.', 'file': 'backend/api/Dockerfile'},
                {'name': 'media', 'context': '.', 'file': 'backend/media/Dockerfile'}),
    'web': ({'name': 'web', 'context': 'web', 'file': 'web/Dockerfile'},),
}


def touches(path, prefixes):
    return any(path == prefix or (prefix.endswith('/') and path.startswith(prefix))
               for prefix in prefixes)


def areas(paths):
    """The set of areas the changed paths touch."""
    paths = [path for path in paths if path and not touches(path, NO_AREA)]
    return {area for area, prefixes in AREAS.items()
            if any(touches(path, prefixes) for path in paths)}


def outputs(touched):
    """`name=value` lines for $GITHUB_OUTPUT."""
    lines = [f'{area}={"true" if area in touched else "false"}' for area in AREAS]
    images = [image for area, built in IMAGES.items() if area in touched for image in built]
    lines.append('images=' + json.dumps(images, separators=(',', ':')))
    return lines


def main(argv):
    touched = set(AREAS) if '--all' in argv else areas(sys.stdin.read().splitlines())
    print('\n'.join(outputs(touched)))
    print('areas: ' + (', '.join(sorted(touched)) or 'none'), file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
