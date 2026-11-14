#!/usr/bin/env python3
"""Stop hook: runs the fast checks of the areas with uncommitted or unpushed
changes before Claude ends its turn. Exit code 2 sends Claude back to fix
what failed (stderr shows why). Checks whose tools are missing are skipped,
and the slow ones (clippy, test suites) stay with CI and AGENTS.md.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

TAIL = 60  # lines of output per failed check


def run(args, cwd, timeout=300):
    try:
        result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as error:
        return None, str(error)
    return result.returncode, (result.stdout + result.stderr).strip()


def changed_files(root):
    """Paths changed in the working tree, staged, untracked or in commits no remote has."""
    paths = set()
    for args in (['git', 'diff', '--name-only', 'HEAD'],
                 ['git', 'ls-files', '--others', '--exclude-standard'],
                 ['git', 'log', '--name-only', '--format=', 'HEAD', '--not', '--remotes']):
        code, out = run(args, root, timeout=30)
        if code == 0:
            paths.update(line for line in out.splitlines() if line)
    return sorted(path for path in paths if (root / path).exists())


def checks(root, paths):
    """(name, command, working directory) of each check the changed paths need."""
    found = []
    has_rustfmt = shutil.which('cargo') and run(['cargo', 'fmt', '--version'], root, 30)[0] == 0
    for workspace in ('backend', 'desktop'):
        if has_rustfmt and any(p.startswith(workspace + '/') and p.endswith('.rs') for p in paths):
            found.append((f'{workspace}: cargo fmt',
                          ['cargo', 'fmt', '--manifest-path', f'{workspace}/Cargo.toml', '--all', '--', '--check', '--color', 'never'],
                          root))
    web = root / 'web'
    scripts = [p[len('web/'):] for p in paths
               if p.startswith('web/') and p.endswith(('.ts', '.tsx', '.js', '.mjs', '.cjs', '.jsx'))]
    if scripts and (web / 'node_modules').is_dir() and shutil.which('node'):
        found.append(('web: eslint',
                      ['node', '--require', './eslint-ts6-register.cjs', './node_modules/eslint/bin/eslint.js',
                       '--no-warn-ignored', *scripts], web))
        if any(p.endswith(('.ts', '.tsx')) for p in scripts):
            found.append(('web: tsc', ['node', './node_modules/typescript/bin/tsc', '--noEmit'], web))
    if any(p.startswith('.github/workflows/') for p in paths):
        found.append(('workflows: action pins',
                      [sys.executable, '-m', 'unittest', '-q', 'test_workflow_pins'],
                      root / 'deploy/compose/tests'))
    return found


def main():
    try:
        json.load(sys.stdin)
    except ValueError:
        pass
    root = Path(os.environ.get('CLAUDE_PROJECT_DIR') or '.').resolve()
    failures = []
    for name, args, cwd in checks(root, changed_files(root)):
        code, out = run(args, cwd)
        if code is None:
            continue  # the tool could not start: not this change's fault
        if code != 0:
            failures.append(f'## {name} failed: {" ".join(args)} (in {cwd.relative_to(root) or "."})\n'
                            + '\n'.join(out.splitlines()[-TAIL:]))
    if not failures:
        return 0
    print('Fast checks failed on your changes; fix them before you finish '
          '(.claude/hooks/fast_checks.py):\n\n' + '\n\n'.join(failures), file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main())
