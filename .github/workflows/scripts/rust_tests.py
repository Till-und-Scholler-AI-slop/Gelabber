"""The backend tests a pull request can affect (GEL-393).

ci.yml pipes a pull request's changed files into this script. It maps them to
the backend workspace's crates (read from their Cargo.toml files, so no
Rust toolchain is needed), adds every crate that depends on one of them, and prints a nextest filter for their tests plus the
crates as JSON, as `$GITHUB_OUTPUT` lines.

When in doubt it runs everything: on a push, the merge queue or a manual run;
when a path is a Cargo.toml, Cargo.lock, the toolchain, .github/, a file in
backend/ outside a crate or a file at the top of the repository; and when
the crates cannot be read. A path in another area's folder (web/, desktop/,
…) affects no backend test.

Only the test run shrinks. Cargo builds the whole workspace either way:
building fewer packages unifies different dependency features (`cargo tree`
shows eight crates for `-p gelabber-api` alone), so the cache would miss.
"""
import json
from pathlib import Path, PurePosixPath
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[3]
WORKSPACE = 'backend'
# Paths that change what every crate is built or tested with.
EVERYTHING = ('.github/', 'rust-toolchain.toml')
ALL = 'all()'


def crates(root=None):
    """The workspace's crates: name -> (folder relative to the repository,
    the workspace crates it depends on, in any dependency table)."""
    root = root or ROOT
    workspace = tomllib.loads((root / WORKSPACE / 'Cargo.toml').read_text())['workspace']
    found = {}
    for member in workspace['members']:
        if any(char in member for char in '*?['):
            raise ValueError(f'member pattern {member}')
        folder = PurePosixPath(WORKSPACE, member)
        manifest = tomllib.loads((root / folder / 'Cargo.toml').read_text())
        tables = [manifest] + list(manifest.get('target', {}).values())
        deps = {spec.get('package', name) if isinstance(spec, dict) else name
                for table in tables
                for kind in ('dependencies', 'dev-dependencies', 'build-dependencies')
                for name, spec in table.get(kind, {}).items()}
        found[manifest['package']['name']] = (folder, deps)
    return {name: (folder, deps & set(found)) for name, (folder, deps) in found.items()}


def crate_of(path, workspace):
    """The crate a changed path belongs to, or None when the path can affect
    every crate (or none can be named for it)."""
    for name, (folder, _) in workspace.items():
        if path.startswith(f'{folder}/'):
            # A manifest can change features that the whole workspace unifies.
            return None if PurePosixPath(path).name == 'Cargo.toml' else name
    return None


def affected(paths, workspace):
    """The crates whose tests the changed paths can affect, or None for all."""
    paths = [path for path in paths if path]
    if not paths:
        return None
    changed = set()
    for path in paths:
        if path.startswith(EVERYTHING) or '/' not in path:
            return None
        if not path.startswith(f'{WORKSPACE}/'):
            continue
        crate = crate_of(path, workspace)
        if crate is None:
            return None
        changed.add(crate)
    if not changed:
        return None
    # Add the crates that depend on a changed one, until nothing is added.
    while True:
        more = {name for name, (_, deps) in workspace.items() if deps & changed} - changed
        if not more:
            return changed
        changed |= more


def outputs(selected):
    """`name=value` lines for $GITHUB_OUTPUT: `all`, the nextest filter and
    the selected crates as JSON (empty when all run)."""
    names = [] if selected is None else sorted(selected)
    expression = ALL if selected is None else ' | '.join(f'package({name})' for name in names)
    return [f'all={"true" if selected is None else "false"}', f'filter={expression}',
            'packages=' + json.dumps(names, separators=(',', ':'))]


def main(argv):
    event = argv[argv.index('--event') + 1] if '--event' in argv else ''
    paths = sys.stdin.read().splitlines()
    selected = None
    if event == 'pull_request':
        try:
            selected = affected(paths, crates())
        except (OSError, KeyError, TypeError, ValueError) as error:
            print(f'cannot place the changes ({error}): all tests', file=sys.stderr)
    print('\n'.join(outputs(selected)))
    print('backend tests: ' + ('all' if selected is None else ', '.join(sorted(selected))), file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
