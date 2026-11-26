"""The tree a pull request's CI run passed, and its reuse on `main` (ci.yml).

A pull request's run tests the merge commit refs/pull/N/merge and records
its tree (`record`). When that pull request is merged into `main` and
`main` has not moved since the run, the merge commit on `main` has the same
tree, so CI on the push need not check it again: `find` prints the run
whose result stands for it. Anything else (no pull request, another tree,
fewer areas checked, an error) prints nothing, and CI runs every check.

`find --candidate` finds the same for a release tag, from a run that also
packaged the apps as the release ships them (desktop-native.yml `candidate`):
desktop-release.yml signs those instead of building again (GEL-358).
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ARTIFACT = 'tested-tree'
WORKFLOW = '.github/workflows/ci.yml'
# The areas a run must have checked to stand for a push, which checks all.
AREAS = ('backend', 'web', 'desktop', 'deploy')


def git(*args):
    return subprocess.check_output(('git', *args), text=True).strip()


def gh(*args):
    return subprocess.check_output(('gh', *args), text=True)


def record(areas, sha, tree, candidate=False):
    """What a pull request's run passed, as stored in the artifact."""
    return {'schema': 1, 'sha': sha, 'tree': tree, 'candidate': candidate,
            'areas': sorted(area for area, touched in areas.items() if touched == 'true')}


def merged_pull(pulls, sha, repo):
    """The pull request from this repository whose merge into main made `sha`."""
    for pull in pulls:
        if (pull.get('merged_at') and pull.get('merge_commit_sha') == sha
                and pull['base']['ref'] == 'main'
                and pull['head']['repo'] and pull['head']['repo']['full_name'] == repo):
            return pull
    return None


def candidate_runs(runs, pull, repo):
    """Successful CI runs of the pull request's last head, newest first."""
    found = [run for run in runs
             if run['path'] == WORKFLOW and run['event'] == 'pull_request'
             and run['status'] == 'completed' and run['conclusion'] == 'success'
             and run['head_sha'] == pull['head']['sha']
             and run['head_repository']['full_name'] == repo
             and run['repository']['full_name'] == repo]
    return sorted(found, key=lambda run: run['run_started_at'], reverse=True)


def stands_for(tested, tree, candidate=False):
    """Whether a run's record covers a push of `tree` (all areas, same tree),
    and with `candidate` also its release packages."""
    return (tested.get('schema') == 1 and tested.get('tree') == tree
            and set(AREAS) <= set(tested.get('areas', ()))
            and (tested.get('candidate') is True or not candidate))


def find(repo, sha, tree, candidate=False):
    pulls = json.loads(gh('api', f'repos/{repo}/commits/{sha}/pulls'))
    pull = merged_pull(pulls, sha, repo)
    if pull is None:
        print(f'{sha} is no merge of a pull request from {repo}', file=sys.stderr)
        return None
    runs = json.loads(gh('api', '--paginate', '--slurp',
                         f'repos/{repo}/actions/runs?event=pull_request&status=success'
                         f'&head_sha={pull["head"]["sha"]}&per_page=100'))
    for run in candidate_runs([run for page in runs for run in page['workflow_runs']], pull, repo):
        with tempfile.TemporaryDirectory() as folder:
            try:
                gh('run', 'download', str(run['id']), '--repo', repo, '--name', ARTIFACT, '--dir', folder)
            except subprocess.CalledProcessError:
                print(f'run {run["id"]} has no {ARTIFACT}', file=sys.stderr)
                continue
            tested = json.loads((Path(folder) / f'{ARTIFACT}.json').read_text())
        if stands_for(tested, tree, candidate):
            print(f'run {run["id"]} of #{pull["number"]} passed tree {tree}', file=sys.stderr)
            return run['id']
        print(f'run {run["id"]} of #{pull["number"]} passed {tested.get("tree")} '
              f'with {tested.get("areas")} (candidate: {tested.get("candidate")}), '
              f'not {tree} with every area', file=sys.stderr)
    return None


def main(argv):
    if argv[:1] == ['record']:
        record_ = record(json.loads(os.environ['AREAS']), os.environ['GITHUB_SHA'], git('rev-parse', 'HEAD^{tree}'),
                         os.environ.get('CANDIDATE') == 'true')
        Path(f'{ARTIFACT}.json').write_text(json.dumps(record_, indent=2) + '\n')
        print(json.dumps(record_))
    elif argv[:1] == ['find']:
        try:
            run = find(os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_SHA'], git('rev-parse', 'HEAD^{tree}'),
                       '--candidate' in argv)
        except Exception as error:  # noqa: BLE001 - any doubt runs every check
            print(f'::warning::no reuse, running every check: {error}', file=sys.stderr)
            run = None
        print(f'run={run or ""}')
    else:
        sys.exit('usage: tested_tree.py record|find')


if __name__ == '__main__':
    main(sys.argv[1:])
