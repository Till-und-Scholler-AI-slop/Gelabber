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

Read as text, without a YAML library, as test_workflow_pins.py does.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / '.github/workflows'
WRITE = re.compile(r'^[ \t]*(?:[a-z-]+:[ \t]*write|permissions:[ \t]*write-all)[ \t]*$', re.M)
SKIPS_PULL_REQUESTS = "github.event_name != 'pull_request'"
LOCAL_CALL = re.compile(r'^    uses:\s*\./\.github/workflows/(\S+)\s*$', re.M)
CARGO = re.compile(r'\bcargo (?:build|test|clippy|run|check|nextest)\b')


def section(text, key, indent):
    """The block under `key:` at `indent` spaces, without the key line."""
    found = re.search(rf'^{" " * indent}{re.escape(key)}:[^\n]*\n((?:(?:{" " * (indent + 1)}[^\n]*|\s*)\n)*)',
                      text + '\n', re.M)
    return found.group(1) if found else ''


def jobs(text):
    """Each job's lines, by job id."""
    body = text.split('\njobs:\n', 1)[1]
    found = {}
    for match in re.finditer(r'^  ([A-Za-z0-9_-]+):\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)', body, re.M | re.S):
        found[match.group(1)] = match.group(2)
    return found


def job_if(job):
    """A job's `if:`, also when it spans lines (`>-`)."""
    found = re.search(r'^    if:\s*(?:[>|]-?)?(.*\n(?:      .*\n)*)', job, re.M)
    return ' '.join(found.group(1).split()) if found else ''


def runs_on_pull_requests(text):
    return re.search(r'^  pull_request(_target)?:', section(text, 'on', 0), re.M) is not None


def read(name):
    return (WORKFLOWS / name).read_text()


def pull_request_jobs(text):
    """The jobs of a workflow that a pull request can run, by job id."""
    return {job: body for job, body in jobs(text).items() if SKIPS_PULL_REQUESTS not in job_if(body)}


def pull_request_workflows():
    """Workflows a pull request starts, and those their pull request jobs
    call, by file name."""
    todo = [path.name for path in sorted(WORKFLOWS.glob('*.yml')) if runs_on_pull_requests(path.read_text())]
    seen = []
    while todo:
        name = todo.pop(0)
        if name in seen:
            continue
        seen.append(name)
        for body in pull_request_jobs(read(name)).values():
            todo.extend(LOCAL_CALL.findall(body))
    return seen


def checkout_steps(text):
    """The lines of each actions/checkout step, by line number."""
    lines = text.splitlines()
    found = {}
    for number, line in enumerate(lines):
        if not re.match(r'\s*(?:-\s+)?uses:\s*actions/checkout@', line):
            continue
        start = number
        while not re.match(r'\s*- ', lines[start]):
            start -= 1
        dash = len(lines[start]) - len(lines[start].lstrip())
        end = start + 1
        while end < len(lines) and (not lines[end].strip() or (
                len(lines[end]) - len(lines[end].lstrip()) > dash)):
            end += 1
        found[number + 1] = '\n'.join(lines[start:end])
    return found


def run_scripts(text):
    """(line number, line) of every line of every `run:` script."""
    lines = text.splitlines()
    found = []
    index = 0
    while index < len(lines):
        run = re.match(r'^(\s*)(?:-\s+)?run:\s*(.*)$', lines[index])
        index += 1
        if not run:
            continue
        if run.group(2).strip() not in ('|', '>', '|-', '>-'):
            found.append((index, run.group(2)))
            continue
        indent = len(run.group(1))
        while index < len(lines) and (not lines[index].strip() or (
                len(lines[index]) - len(lines[index].lstrip()) > indent)):
            found.append((index + 1, lines[index]))
            index += 1
    return found


class Helpers(unittest.TestCase):
    SAMPLE = '\n'.join([
        'on:',
        '  pull_request:',
        '  push:',
        'permissions:',
        '  contents: read',
        'jobs:',
        '  build:',
        '    if: >-',
        "      github.event_name != 'pull_request' &&",
        '      always()',
        '    permissions:',
        '      contents: write',
        '    steps:',
        '      - name: Checkout',
        '        uses: actions/checkout@0000000000000000000000000000000000000000 # v1',
        '        with:',
        '          persist-credentials: false',
        '      - uses: actions/checkout@0000000000000000000000000000000000000000 # v1',
        '      - run: echo ${{ github.ref }}',
        '      - run: |',
        '          cargo test',
        '',
        '          echo done',
        '  call:',
        '    uses: ./.github/workflows/other.yml',
        '',
    ])

    def test_reads_jobs_conditions_and_permissions(self):
        found = jobs(self.SAMPLE)
        self.assertEqual(list(found), ['build', 'call'])
        self.assertEqual(job_if(found['build']), "github.event_name != 'pull_request' && always()")
        self.assertEqual(WRITE.findall(section(found['build'], 'permissions', 4)), ['      contents: write'])
        self.assertEqual(WRITE.findall(section(self.SAMPLE, 'permissions', 0)), [])
        self.assertTrue(runs_on_pull_requests(self.SAMPLE))
        self.assertFalse(runs_on_pull_requests('on:\n  push:\njobs:\n'))
        self.assertEqual(LOCAL_CALL.findall(found['call']), ['other.yml'])

    def test_reads_checkouts_and_scripts(self):
        steps = checkout_steps(self.SAMPLE)
        self.assertEqual(sorted(steps), [15, 18])
        self.assertIn('persist-credentials: false', steps[15])
        self.assertNotIn('persist-credentials', steps[18])
        self.assertEqual([line.strip() for _, line in run_scripts(self.SAMPLE) if line.strip()],
                         ['echo ${{ github.ref }}', 'cargo test', 'echo done'])


class PullRequestTokens(unittest.TestCase):
    def test_the_walk_finds_ci_and_what_it_calls(self):
        names = pull_request_workflows()
        for name in ('ci.yml', 'e2e-extended.yml', 'desktop-native.yml', 'minio-images.yml'):
            with self.subTest(name=name):
                self.assertIn(name, names)

    def test_pull_request_jobs_get_read_only_tokens(self):
        found = []
        for name in pull_request_workflows():
            text = read(name)
            if WRITE.search(section(text, 'permissions', 0)):
                found.append(f'{name}: top-level permissions')
            for job, body in pull_request_jobs(text).items():
                if WRITE.search(section(body, 'permissions', 4)):
                    found.append(f'{name}: job {job}')
        self.assertEqual(found, [], 'These run on pull requests with a write token; '
                         f'grant writes only to jobs whose `if:` has {SKIPS_PULL_REQUESTS}')

    def test_workflows_a_pull_request_calls_ask_for_no_write(self):
        # GitHub checks a call against the caller's grant before any `if:`.
        found = []
        for name in pull_request_workflows():
            for job, body in pull_request_jobs(read(name)).items():
                call = LOCAL_CALL.search(body)
                if call and WRITE.search(read(call.group(1))):
                    found.append(f'{name}: job {job} calls {call.group(1)}, which asks for a write')
        self.assertEqual(found, [])

    def test_merge_queue_jobs_get_read_only_tokens(self):
        # A merge queue run builds code that has not landed yet, as a pull request's does (GEL-362).
        found = []
        for name in pull_request_workflows():
            text = read(name)
            if not re.search(r'^  merge_group:', section(text, 'on', 0), re.M):
                continue
            for job, body in jobs(text).items():
                if WRITE.search(section(body, 'permissions', 4)) and \
                        "github.event_name != 'merge_group'" not in job_if(body):
                    found.append(f'{name}: job {job}')
        self.assertEqual(found, [], "grant writes only to jobs whose `if:` also has "
                         "github.event_name != 'merge_group'")

    def test_checkouts_keep_no_credentials(self):
        found = [f'{name}:{number}' for name in pull_request_workflows()
                 for number, step in checkout_steps(read(name)).items()
                 if not re.search(r'^\s*persist-credentials:\s*false\s*$', step, re.M)]
        self.assertEqual(found, [], 'Check out with `persist-credentials: false`, so build code '
                         'cannot read the token from .git/config')


class PushPaths(unittest.TestCase):
    """Pushes and releases still write, from jobs of their own."""

    def test_libwebrtc_packages_are_stored_after_pushes_and_releases(self):
        native = read('desktop-native.yml')
        for system in ('linux', 'windows'):
            with self.subTest(system=system):
                self.assertIn(f'      libwebrtc-{system}-store:\n'
                              f'        value: ${{{{ jobs.libwebrtc-{system}.outputs.store }}}}\n', native)
                build = jobs(native)[f'libwebrtc-{system}']
                self.assertIn("store: ${{ steps.asset.outputs.missing == 'true' && steps.key.outputs.key || '' }}",
                              build)
                self.assertNotIn('gh release upload', build)
                store = jobs(read('libwebrtc-store.yml'))[system]
                self.assertIn('      contents: write\n', section(store, 'permissions', 4))
                self.assertIn('gh release upload libwebrtc-packages', store)
        for name, needs in (('ci.yml', 'desktop'), ('desktop-release.yml', 'build')):
            with self.subTest(name=name):
                store = jobs(read(name))['libwebrtc-store']
                self.assertIn('    uses: ./.github/workflows/libwebrtc-store.yml\n', store)
                self.assertIn(f'    needs: {needs}\n', store)
                self.assertIn('      contents: write\n', section(store, 'permissions', 4))
                for system in ('linux', 'windows'):
                    self.assertIn(f'      {system}-key: ${{{{ needs.{needs}.outputs.libwebrtc-{system}-store }}}}\n',
                                  store)
        self.assertIn('libwebrtc-store]', jobs(read('ci.yml'))['ci-ok'])

    def test_minio_images_are_pushed_from_main(self):
        text = read('publish-minio.yml')
        self.assertIn('  push:\n    branches: [main]\n', section(text, 'on', 0))
        images, publish = jobs(text)['images'], jobs(text)['publish']
        self.assertIn('    uses: ./.github/workflows/minio-images.yml\n', images)
        self.assertIn("      keep: ${{ github.ref == 'refs/heads/main' }}\n", images)
        self.assertIn("    if: github.ref == 'refs/heads/main' && needs.images.outputs.built == 'true'\n", publish)
        self.assertIn('      packages: write\n', section(publish, 'permissions', 4))
        self.assertIn('          name: minio-images\n', publish)
        self.assertIn('docker push "${GHCR_IMAGE}:${MINIO_RELEASE}"', publish)
        self.assertIn('docker push "${GHCR_MC_IMAGE}:${MC_RELEASE}"', publish)
        self.assertIn('          name: minio-images\n', read('minio-images.yml'))


class RunScripts(unittest.TestCase):
    def test_scripts_read_the_github_context_from_the_environment(self):
        found = [f'{path.name}:{number}: {line.strip()}'
                 for path in sorted(WORKFLOWS.glob('*.yml'))
                 for number, line in run_scripts(path.read_text())
                 if '${{ github.' in line]
        self.assertEqual(found, [], 'Use $GITHUB_REF, $GITHUB_REPOSITORY, … or an `env:` entry')

    def test_desktop_cargo_builds_keep_the_lock_file(self):
        found = [f'desktop-native.yml:{number}: {line.strip()}'
                 for number, line in run_scripts(read('desktop-native.yml'))
                 if CARGO.search(line) and '--locked' not in line]
        self.assertEqual(found, [])


if __name__ == '__main__':
    unittest.main()
