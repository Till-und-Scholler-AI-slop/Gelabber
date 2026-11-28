"""Which jobs of e2e-extended.yml a run starts, and what its issue says.

`plan <event> <before>`: the `faults=` and `desktop=` lines for
$GITHUB_OUTPUT. A push starts the jobs its commits concern (the diff from
`before`, the commit the branch pointed at, fetched from origin); anything
else starts both: runs by hand, release and hotfix pull requests, and pushes
without a known starting point (a new branch, a commit origin no longer
has). By hand, the changed paths on stdin give the same lines, and `--all`
starts both.

`result <plan> <faults> <desktop-live>`: the needed jobs' results give the
push run's outcome for its issue: `failure` when one failed or was cancelled
(a timeout; the job that reports does not run when the whole run was
cancelled), `success` when every needed job ran green, otherwise `other`:
a run that skipped a job says nothing about it, so it neither opens nor
closes the issue.
"""
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changes import touches  # noqa: E402

WORKFLOW = ('.github/workflows/e2e-extended.yml', '.github/workflows/scripts/e2e_jobs.py',
            '.github/workflows/scripts/changes.py', '.github/workflows/scripts/apt-install.sh',
            # Both jobs start their MinIO and browsers through these.
            '.github/workflows/scripts/start-minio.sh', '.github/workflows/scripts/playwright-browsers.sh',
            # The MinIO both jobs start is pinned there.
            'deploy/compose/compose.yaml',
            # Both jobs' cargo builds follow it.
            'rust-toolchain.toml')
JOBS = {
    # The web client in browsers against the API and the SFU.
    'faults': ('backend/', 'web/') + WORKFLOW,
    # The desktop app against the API and the SFU, on the parity stack: its
    # seed is a web script, the call's people use the web client, and every
    # request goes through Vite's proxy.
    'desktop': ('desktop/', 'backend/', 'web/') + WORKFLOW,
}


def plan(paths):
    """The jobs the changed paths start."""
    return {job for job, prefixes in JOBS.items()
            if any(path and touches(path, prefixes) for path in paths)}


def pushed_paths(before):
    """The paths changed since the commit `before`, or None when it is unknown here."""
    if not re.fullmatch(r'[0-9a-f]{40}', before) or before == '0' * 40:
        return None
    git = ['git', '-c', 'core.quotePath=false']
    if subprocess.run(git + ['fetch', '--quiet', '--depth=1', 'origin', before]).returncode != 0:
        return None
    if subprocess.run(git + ['cat-file', '-e', f'{before}^{{commit}}']).returncode != 0:
        return None
    diff = subprocess.run(git + ['diff', '--no-renames', '--name-only', before, 'HEAD'],
                          check=True, capture_output=True, text=True)
    return diff.stdout.splitlines()


def result(*results):
    """The push run's outcome from the results of the jobs it needed."""
    if any(r in ('failure', 'cancelled') for r in results):
        return 'failure'
    return 'success' if all(r == 'success' for r in results) else 'other'


def main(argv):
    if argv[:1] == ['result']:
        print(result(*argv[1:]))
        return
    if argv[:1] == ['plan']:
        event, before = argv[1:3]
        paths = pushed_paths(before) if event == 'push' else None
        jobs = set(JOBS) if paths is None else plan(paths)
    elif '--all' in argv:
        jobs = set(JOBS)
    else:
        jobs = plan(sys.stdin.read().splitlines())
    for job in JOBS:
        print(f'{job}={"true" if job in jobs else "false"}')
    print('jobs: ' + (', '.join(sorted(jobs)) or 'none'), file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
