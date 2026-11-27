"""What a pull request's CI run can write, and what its build code can read
(GEL-49).

A pull request runs code from outside: its own changes, crates, npm
packages, gclient hooks and Go modules. Its jobs therefore get read-only
tokens, writes (libwebrtc packages, images) run in jobs that pull requests
skip, and a checkout leaves no token in `.git/config` for that code to read.
GitHub caps a reusable workflow at its caller's permissions, so a workflow a
pull request calls asks for no write at all. Expressions in `run:` scripts
are pasted in before the shell parses them, so scripts read the github
context from the environment.

zizmor checks part of this too (template-injection, artipacked), but only
for contexts and steps it judges dangerous; these rules hold everywhere.
"""
import re
import unittest

from workflow_data import load, local_call, names, scripts, skips, steps, triggers, writes

CARGO = re.compile(r'\bcargo (?:build|test|clippy|run|check|nextest)\b')
UNTRUSTED = ('pull_request', 'merge_group')


def untrusted_jobs(name, event):
    """The jobs of a workflow that a run for `event` can start, by job id."""
    return {job_id: job for job_id, job in load(name).get('jobs', {}).items() if not skips(job, event)}


def pull_request_workflows(event='pull_request'):
    """Workflows `event` starts, and those their jobs call, by file name."""
    todo = [name for name in names() if event in triggers(load(name))
            or (event == 'pull_request' and 'pull_request_target' in triggers(load(name)))]
    seen = []
    while todo:
        name = todo.pop(0)
        if name not in seen:
            seen.append(name)
            todo += [call for job in untrusted_jobs(name, event).values() if (call := local_call(job))]
    return seen


class PullRequestTokens(unittest.TestCase):
    def test_the_walk_finds_ci_and_what_it_calls(self):
        self.assertEqual(set(pull_request_workflows()),
                         {'ci.yml', 'e2e-extended.yml', 'desktop-native.yml', 'minio-images.yml'})
        self.assertEqual(set(pull_request_workflows('merge_group')),
                         {'ci.yml', 'desktop-native.yml', 'minio-images.yml'})

    def test_untrusted_jobs_get_read_only_tokens(self):
        # A merge queue run builds code that has not landed yet, as a pull request's does (GEL-362).
        for event in UNTRUSTED:
            found = []
            for name in pull_request_workflows(event):
                if writes(load(name).get('permissions')):
                    found.append(f'{name}: top-level permissions')
                found += [f'{name}: job {job_id}' for job_id, job in untrusted_jobs(name, event).items()
                          if writes(job.get('permissions'))]
            with self.subTest(event=event):
                self.assertEqual(found, [], f'These run for {event} with a write token; grant '
                                 f'writes only to jobs whose `if:` skips {event}')

    def test_workflows_a_pull_request_calls_ask_for_no_write(self):
        # GitHub checks a call against the caller's grant before any `if:`.
        found = []
        for name in pull_request_workflows():
            for job_id, job in load(name)['jobs'].items():
                call = local_call(job)
                if call and job_id in untrusted_jobs(name, 'pull_request'):
                    called = load(call)
                    if writes(called.get('permissions')) or any(
                            writes(inner.get('permissions')) for inner in called['jobs'].values()):
                        found.append(f'{name}: job {job_id} calls {call}, which asks for a write')
        self.assertEqual(found, [])

    def test_checkouts_keep_no_credentials(self):
        found = [f'{name}: job {job_id}: {step.get("name", step["uses"])}'
                 for name in pull_request_workflows() for job_id, step in steps(load(name))
                 if step.get('uses', '').startswith('actions/checkout@')
                 and (step.get('with') or {}).get('persist-credentials') is not False]
        self.assertEqual(found, [], 'Check out with `persist-credentials: false`, so build code '
                         'cannot read the token from .git/config')


class PushPaths(unittest.TestCase):
    """Pushes and releases still write, from jobs that build nothing."""

    def test_libwebrtc_packages_are_uploaded_by_a_job_that_builds_nothing(self):
        for name in names():
            for job_id, _, script in scripts(load(name)):
                if 'gh release upload' in script and 'libwebrtc-packages' in script:
                    with self.subTest(name=name, job=job_id):
                        self.assertEqual(name, 'libwebrtc-store.yml')
        store = load('libwebrtc-store.yml')['jobs']
        self.assertEqual(set(store), {'linux', 'windows'})
        for job_id, job in store.items():
            with self.subTest(job=job_id):
                self.assertEqual(writes(job.get('permissions')), {'contents'})
                self.assertFalse([script for _, _, script in scripts({'jobs': {job_id: job}})
                                  if 'build-libwebrtc' in script])
        for name in ('ci.yml', 'desktop-release.yml'):
            with self.subTest(name=name):
                caller = load(name)['jobs']['libwebrtc-store']
                self.assertEqual(local_call(caller), 'libwebrtc-store.yml')
                self.assertEqual(writes(caller.get('permissions')), {'contents'})

    def test_minio_images_are_pushed_from_main_by_a_job_that_builds_nothing(self):
        workflow = load('publish-minio.yml')
        self.assertEqual(workflow['on']['push']['branches'], ['main'])
        images, publish = workflow['jobs']['images'], workflow['jobs']['publish']
        self.assertEqual(local_call(images), 'minio-images.yml')
        self.assertFalse(writes(images.get('permissions')))
        self.assertEqual(writes(publish.get('permissions')), {'packages'})
        self.assertTrue(skips(publish, 'pull_request'))
        self.assertNotIn('uses', publish)
        self.assertFalse([step for step in publish['steps']
                          if step.get('uses', '').startswith('docker/build-push-action@')])
        pushes = [script for _, _, script in scripts({'jobs': {'publish': publish}}) if 'docker push' in script]
        self.assertTrue(pushes)


class RunScripts(unittest.TestCase):
    def test_scripts_read_the_github_context_from_the_environment(self):
        found = [f'{name}: job {job_id}: {step}' for name in names()
                 for job_id, step, script in scripts(load(name)) if '${{ github.' in script]
        self.assertEqual(found, [], 'Use $GITHUB_REF, $GITHUB_REPOSITORY, … or an `env:` entry')

    def test_desktop_cargo_builds_keep_the_lock_file(self):
        found = [f'job {job_id}: {line.strip()}'
                 for job_id, _, script in scripts(load('desktop-native.yml'))
                 for line in script.splitlines() if CARGO.search(line) and '--locked' not in line]
        self.assertEqual(found, [])


class Conditions(unittest.TestCase):
    """`skips` reads a job's `if:` the way GitHub evaluates it for an event."""

    def test_an_equally_safe_condition_counts(self):
        for condition in ("github.event_name != 'pull_request'", "github.event_name == 'push'",
                          "${{ github.ref == 'refs/heads/main' && always() }}",
                          "(github.event_name == 'push' || github.event_name == 'release') && x"):
            with self.subTest(condition=condition):
                self.assertTrue(skips({'if': condition}, 'pull_request'))

    def test_a_condition_a_pull_request_can_meet_does_not(self):
        for condition in ('always()', "github.event_name == 'push' || github.event_name == 'pull_request'",
                          "github.event_name != 'merge_group'", "github.ref != 'refs/heads/main'",
                          "github.ref == 'refs/heads/gh-readonly-queue/v0.14/pr-1'", None):
            with self.subTest(condition=condition):
                self.assertFalse(skips({'if': condition} if condition else {}, 'pull_request'))


if __name__ == '__main__':
    unittest.main()
