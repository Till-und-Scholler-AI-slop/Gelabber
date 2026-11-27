"""Which jobs of e2e-extended.yml a run starts, and what its issue says.

`plan`: the changed paths on stdin give `faults=` and `desktop=` lines for
$GITHUB_OUTPUT; `plan --all` (runs by hand, release and hotfix pull
requests, pushes without a known starting point) starts both.

`result <plan> <faults> <desktop-live>`: the needed jobs' results give the
push run's outcome for its issue: `failure` when one failed or was cancelled
(a timeout; the job that reports does not run when the whole run was
cancelled), `success` when none did and at least one ran green, otherwise
`other`. A skipped job counts for neither.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changes import touches  # noqa: E402

WORKFLOW = ('.github/workflows/e2e-extended.yml', '.github/workflows/scripts/e2e_jobs.py',
            '.github/workflows/scripts/changes.py', '.github/workflows/scripts/apt-install.sh',
            # The MinIO both jobs start is pinned there.
            'deploy/compose/compose.yaml')
JOBS = {
    # The web client in browsers against the API and the SFU.
    'faults': ('backend/', 'web/') + WORKFLOW,
    # The desktop app against the API and the SFU, on the parity stack: its
    # seed and the call's people are web scripts.
    'desktop': ('desktop/', 'backend/', 'web/scripts/parity/', 'web/package.json',
                'web/package-lock.json') + WORKFLOW,
}


def plan(paths):
    """The jobs the changed paths start."""
    return {job for job, prefixes in JOBS.items()
            if any(path and touches(path, prefixes) for path in paths)}


def result(*results):
    """The push run's outcome from the results of the jobs it needed."""
    if any(r in ('failure', 'cancelled') for r in results):
        return 'failure'
    return 'success' if 'success' in results[1:] else 'other'


def main(argv):
    if argv[:1] == ['result']:
        print(result(*argv[1:]))
        return
    jobs = set(JOBS) if '--all' in argv else plan(sys.stdin.read().splitlines())
    for job in JOBS:
        print(f'{job}={"true" if job in jobs else "false"}')
    print('jobs: ' + (', '.join(sorted(jobs)) or 'none'), file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv[1:])
