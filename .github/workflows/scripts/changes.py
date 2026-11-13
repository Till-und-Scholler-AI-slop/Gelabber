"""Which CI areas a pull request touches.

ci.yml pipes the pull request's changed files into this script and starts
only the jobs of the areas it prints. A change under .github/ starts every
area; `--all` (pushes, manual runs) does the same. The prefixes are the one
list of what belongs to an area, so the deployment tests ask this script too.
"""
import json
import sys

AREAS = {
    'backend': ('backend/', 'rust-toolchain.toml', '.dockerignore'),
    # The emoji data is generated into both the backend and the web client.
    'web': ('web/', 'backend/shared/data/'),
    # The legacy import's test reads the web client's storage keys (legacy.rs).
    'desktop': ('desktop/', 'rust-toolchain.toml', 'web/src/voice/settings.ts',
                'web/src/interface/preferences.ts', 'web/src/voice/userVolume.ts',
                'web/src/messages/drafts.ts',
                # theme.rs checks its tokens against the web's (#561).
                'web/src/theme/tokens.fixture.json'),
    # The deployment tests also read the backend's build, both versions and
    # the workflows.
    'deploy': ('deploy/', 'backend/', 'desktop/Cargo.toml', 'rust-toolchain.toml', '.dockerignore'),
    'minio': ('deploy/compose/minio/', 'deploy/compose/mc/', 'deploy/compose/minio-init.sh',
              'deploy/compose/compose.yaml', 'backend/Cargo.toml'),
}
EVERY_AREA = ('.github/',)
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
    paths = [path for path in paths if path]
    if any(touches(path, EVERY_AREA) for path in paths):
        return set(AREAS)
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
