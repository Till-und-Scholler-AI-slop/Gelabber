"""The tree a pull request's CI run passed, and its reuse on `main` (ci.yml).

A pull request's run tests the merge commit refs/pull/N/merge and records
its tree (`record`). When that pull request is merged into `main` and
`main` has not moved since the run, the merge commit on `main` has the same
tree, so CI on the push need not check it again: `find` prints the run
whose result stands for it. Anything else (no pull request, another tree,
fewer areas checked, an error) prints nothing, and CI runs the areas changed
since the last passed push run (`since`).

`find --candidate` finds the same for a release tag, from a run that also
packaged the apps as the release ships them (desktop-native.yml `candidate`):
desktop-release.yml signs those instead of building again (GEL-358).

`find-queue` does it for a push to a version branch, from the merge queue's
run (GEL-362): the same tree, tested on the push's previous head, so the
areas it checked are the push's. A push that changes a cache input
(CACHE_INPUTS) runs its areas anyway, because only pushes save caches.
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
# What the cache keys hash: lock files and manifests, the toolchain, the
# backend's build image and the libwebrtc build (ci.yml, desktop-native.yml).
CACHE_INPUTS = ('Cargo.lock', 'Cargo.toml', 'package-lock.json', 'rust-toolchain.toml')
CACHE_INPUT_PREFIXES = ('backend/media/Dockerfile', 'backend/build/', 'desktop/native/libwebrtc',
                        'desktop/native/scripts/build-libwebrtc', 'desktop/native/patches/', '.github/')
QUEUE_RUNS = 30


def git(*args):
    return subprocess.check_output(('git', *args), text=True).strip()


def gh(*args):
    return subprocess.check_output(('gh', *args), text=True)


def record(areas, sha, tree, candidate=False, base=None):
    """What a pull request's or merge queue's run passed, as stored in the
    artifact; `base` is the commit it was tested on."""
    return {'schema': 1, 'sha': sha, 'tree': tree, 'candidate': candidate, 'base': base,
            'areas': sorted(area for area, touched in areas.items() if touched == 'true')}


def touches_cache_inputs(paths):
    """Whether a change alters what a cache key hashes."""
    return any(path.rsplit('/', 1)[-1] in CACHE_INPUTS or path.startswith(CACHE_INPUT_PREFIXES)
               for path in paths if path)


def queue_runs(runs, repo, sha):
    """Successful CI runs of the merge queue, the one of `sha` first."""
    found = [run for run in runs
             if run['path'] == WORKFLOW and run['event'] == 'merge_group'
             and run['status'] == 'completed' and run['conclusion'] == 'success'
             and run['head_repository']['full_name'] == repo
             and run['repository']['full_name'] == repo]
    found.sort(key=lambda run: run['run_started_at'], reverse=True)
    return sorted(found, key=lambda run: run['head_sha'] != sha)


def stands_for_push(tested, tree, before):
    """Whether a queue run's record covers a push of `tree` on `before`."""
    return (tested.get('schema') == 1 and tested.get('tree') == tree
            and tested.get('base') == before)


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


def download(repo, run_id, folder):
    gh('run', 'download', str(run_id), '--repo', repo, '--name', ARTIFACT, '--dir', folder)
    return json.loads((Path(folder) / f'{ARTIFACT}.json').read_text())


def last_good(runs, repo, sha, ancestor):
    """The head of the newest successful push run on this branch that is an
    ancestor of `sha` (`ancestor(commit)`), or None."""
    for run in sorted(runs, key=lambda run: run['run_started_at'], reverse=True):
        if (run['path'] == WORKFLOW and run['event'] == 'push' and run['status'] == 'completed'
                and run['conclusion'] == 'success' and run['repository']['full_name'] == repo
                and run['head_sha'] != sha and ancestor(run['head_sha'])):
            return run['head_sha']
    return None


def since(repo, branch, sha, before):
    """Where a push's changes start: the last commit whose push run passed.
    GitHub cancels a pending push run when a newer one queues (GEL-363), so
    the previous head alone can leave a push's changes unchecked."""
    runs = json.loads(gh('api', f'repos/{repo}/actions/workflows/ci.yml/runs?event=push&status=success'
                               f'&branch={branch}&per_page=20'))['workflow_runs']

    def ancestor(commit):
        compare = json.loads(gh('api', f'repos/{repo}/compare/{commit}...{sha}'))
        return compare['status'] in ('ahead', 'identical')
    return last_good(runs, repo, sha, ancestor) or before


def find_queue(repo, sha, tree, before, start=None):
    if not before or set(before) == {'0'}:
        print('no previous head', file=sys.stderr)
        return None
    start = start or before
    git('fetch', '--quiet', '--depth=1', 'origin', start)
    paths = git('diff', '--no-renames', '--name-only', start, sha).splitlines()
    if touches_cache_inputs(paths):
        print('the push changes a cache input: its areas run and save their caches', file=sys.stderr)
        return None
    runs = json.loads(gh('api', f'repos/{repo}/actions/runs?event=merge_group&status=success&per_page={QUEUE_RUNS}'))
    for run in queue_runs(runs['workflow_runs'], repo, sha)[:QUEUE_RUNS]:
        with tempfile.TemporaryDirectory() as folder:
            try:
                tested = download(repo, run['id'], folder)
            except (subprocess.CalledProcessError, OSError, ValueError):
                continue
        if stands_for_push(tested, tree, before):
            print(f'queue run {run["id"]} passed tree {tree} on {before}', file=sys.stderr)
            return run['id']
    print(f'no queue run passed tree {tree} on {before}', file=sys.stderr)
    return None


def main(argv):
    if argv[:1] == ['record']:
        base = os.environ.get('BASE_SHA') or git('rev-parse', 'HEAD^1')
        record_ = record(json.loads(os.environ['AREAS']), os.environ['GITHUB_SHA'], git('rev-parse', 'HEAD^{tree}'),
                         os.environ.get('CANDIDATE') == 'true', base)
        Path(f'{ARTIFACT}.json').write_text(json.dumps(record_, indent=2) + '\n')
        print(json.dumps(record_))
    elif argv[:1] == ['find']:
        try:
            run = find(os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_SHA'], git('rev-parse', 'HEAD^{tree}'),
                       '--candidate' in argv)
        except Exception as error:  # noqa: BLE001 - any doubt runs the changed areas
            print(f'::warning::no reuse, running the changed areas: {error}', file=sys.stderr)
            run = None
        print(f'run={run or ""}')
    elif argv[:1] == ['find-queue']:
        try:
            run = find_queue(os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_SHA'],
                             git('rev-parse', 'HEAD^{tree}'), os.environ.get('BEFORE', ''),
                             os.environ.get('SINCE', ''))
        except Exception as error:  # noqa: BLE001 - any doubt runs the changed areas
            print(f'::warning::no reuse, running the changed areas: {error}', file=sys.stderr)
            run = None
        print(f'run={run or ""}')
    elif argv[:1] == ['since']:
        before = os.environ.get('BEFORE', '')
        try:
            start = since(os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_REF_NAME'],
                          os.environ['GITHUB_SHA'], before)
        except Exception as error:  # noqa: BLE001 - the previous head as before GEL-363
            print(f'::warning::no last successful push run found: {error}', file=sys.stderr)
            start = before
        print(f'since={start}')
    else:
        sys.exit('usage: tested_tree.py record|find|find-queue|since')


if __name__ == '__main__':
    main(sys.argv[1:])
