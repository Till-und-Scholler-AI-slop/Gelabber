"""The areas ci.yml runs for a pull request (.github/workflows/scripts/changes.py)."""
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import workflow_data

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/changes.py'


def load():
    spec = importlib.util.spec_from_file_location('changes', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


changes = load()


def changed_areas(*paths):
    """The CI areas ci.yml runs for a pull request changing these paths."""
    return changes.areas(paths)


class Areas(unittest.TestCase):
    def test_each_folder_starts_its_area(self):
        for path, area in (('backend/api/src/main.rs', 'backend'), ('web/src/main.tsx', 'web'),
                           ('desktop/gelabber/src/main.rs', 'desktop'), ('deploy/backup.sh', 'deploy'),
                           ('deploy/compose/minio/Dockerfile', 'minio')):
            with self.subTest(path=path):
                self.assertIn(area, changes.areas([path]))

    def test_a_web_change_leaves_the_backend_alone(self):
        self.assertEqual(changes.areas(['web/src/main.tsx', 'web/package.json']), {'web'})

    def test_a_backend_change_leaves_web_and_desktop_alone(self):
        self.assertEqual(changes.areas(['backend/api/src/main.rs']), {'backend', 'deploy'})

    def test_the_toolchain_concerns_both_rust_workspaces(self):
        self.assertTrue({'backend', 'desktop'} <= changes.areas(['rust-toolchain.toml']))

    def test_emoji_data_concerns_web_too(self):
        self.assertIn('web', changes.areas(['backend/shared/data/emoji-18.0.json']))

    def test_a_github_file_starts_the_areas_whose_jobs_it_runs(self):
        # GEL-390: before, any change under .github/ started every area.
        for path, expected in (
                ('.github/workflows/ci.yml', set(changes.AREAS)),
                ('.github/workflows/desktop-native.yml', {'desktop', 'deploy'}),
                ('.github/workflows/desktop-release.yml', {'desktop', 'deploy'}),
                ('.github/workflows/libwebrtc-store.yml', {'desktop', 'deploy'}),
                ('.github/workflows/scripts/apt-install.sh', {'desktop', 'deploy'}),
                # The rust job picks and checks the upgrade test's release with it (GEL-444).
                ('.github/workflows/scripts/previous_release.sh', {'backend', 'deploy'}),
                ('.github/workflows/minio-images.yml', {'minio', 'deploy'}),
                ('.github/workflows/publish-minio.yml', {'minio', 'deploy'}),
                ('.github/workflows/cache-cleanup.yml', {'deploy'}),
                ('.github/workflows/e2e-extended.yml', {'web', 'deploy'}),
                ('.github/workflows/scripts/caches.py', {'deploy'}),
                ('.github/dependabot.yml', {'deploy'}),
                ('.github/ISSUE_TEMPLATE/bug_report.yml', set()),
                ('.github/release-notes/v0.14.1.md', set())):
            with self.subTest(path=path):
                self.assertEqual(changes.areas([path]), expected)

    def test_every_workflow_starts_deploy_where_it_is_linted(self):
        for path in sorted((ROOT / '.github/workflows').rglob('*')):
            if path.is_file() and '__pycache__' not in path.parts:
                with self.subTest(path=path.name):
                    self.assertIn('deploy', changes.areas([path.relative_to(ROOT).as_posix()]))

    def test_a_called_workflow_starts_the_area_of_the_job_that_calls_it(self):
        jobs = workflow_data.load('ci.yml')['jobs']
        # Gated on another job rather than an area: libwebrtc-store needs desktop.
        not_by_area = {'libwebrtc-store': 'desktop'}
        called = set()
        for job_id, job in jobs.items():
            uses = [job.get('uses', '')] + [step.get('uses', '') for step in job.get('steps', [])]
            for local in (use for use in uses if use.startswith('./')):
                with self.subTest(job=job_id, uses=local):
                    match = re.fullmatch(r'\./(\.github/workflows/[\w.-]+\.yml)', local)
                    self.assertTrue(match, f'{job_id} uses {local}: map it in changes.py and here')
                    gates = re.findall(r"needs\.changes\.outputs\.(\w+) == 'true'", str(job.get('if', '')))
                    if job_id in not_by_area:
                        self.assertEqual(gates, [])
                        self.assertIn(not_by_area[job_id], changes.areas([match.group(1)]))
                        continue
                    self.assertEqual(len(gates), 1, f'{job_id}: no area gate this test understands')
                    self.assertIn(gates[0], changes.areas([match.group(1)]))
                    called.add(gates[0])
        self.assertEqual(called, {'desktop', 'minio'})

    def test_files_other_areas_read_start_them(self):
        # Read by test_public_pages.py, test_update_signature.py,
        # generate-emoji-data.py --check, landingImage.test.mjs,
        # ci-fault-loop.checks.mjs (a step of e2e-extended.yml) and the
        # packaging tests (prerequisites.py). Also the
        # deployment tests' own inputs outside deploy/: the Claude Code hooks,
        # both versions, the backend's build.
        for path, area in (('.claude/hooks/fast_checks.py', 'deploy'),
                           ('backend/Cargo.toml', 'deploy'), ('desktop/Cargo.toml', 'deploy'),
                           ('backend/build/run-native-ci.sh', 'deploy'), ('.dockerignore', 'deploy'),
                           ('.dockerignore', 'backend'), ('backend/build/run-native-ci.sh', 'backend'),
                           ('web/docker-entrypoint.d/50-gelabber-public-pages.sh', 'deploy'),
                           ('desktop/packaging/updater.pub', 'deploy'),
                           ('desktop/ui/assets/emoji/emoji-data.json', 'web'),
                           ('.github/workflows/e2e-extended.yml', 'web'),
                           ('docs/design/v0.10/landing-preview-en.png', 'web'),
                           ('deploy/compose/tests/prerequisites.py', 'desktop')):
            with self.subTest(path=path):
                self.assertTrue((ROOT / path).is_file())
                self.assertIn(area, changes.areas([path]))

    def test_docs_alone_start_nothing(self):
        self.assertEqual(changes.areas(['docs/design-qa.md', 'AGENTS.md', 'README.md', '']), set())

    def test_a_prefix_is_a_folder_not_a_name_start(self):
        self.assertEqual(changes.areas(['backend-notes.md', 'webhook.md']), set())


class Outputs(unittest.TestCase):
    def run_script(self, *args, stdin=''):
        result = subprocess.run([sys.executable, str(SCRIPT), *args], input=stdin,
                                capture_output=True, text=True, check=True)
        return dict(line.split('=', 1) for line in result.stdout.splitlines())

    def test_flags_and_images_for_a_backend_change(self):
        out = self.run_script(stdin='backend/media/src/main.rs\n')
        self.assertEqual(out['backend'], 'true')
        self.assertEqual(out['web'], 'false')
        self.assertEqual([image['name'] for image in json.loads(out['images'])], ['media'])

    def test_only_the_web_image_for_a_web_change(self):
        out = self.run_script(stdin='web/src/main.tsx\n')
        self.assertEqual(json.loads(out['images']), [
            {'name': 'web', 'context': 'web', 'file': 'web/Dockerfile'}])

    def test_no_images_is_an_empty_list(self):
        self.assertEqual(self.run_script(stdin='docs/x.md\n')['images'], '[]')

    def test_minio_is_built_only_when_its_dockerfiles_change(self):
        # GEL-392: otherwise the minio job checks the published pinned images.
        for path, area, build in (('deploy/compose/minio/Dockerfile', 'true', 'true'),
                                  ('deploy/compose/mc/Dockerfile', 'true', 'true'),
                                  ('.github/workflows/minio-images.yml', 'true', 'true'),
                                  ('deploy/compose/compose.yaml', 'true', 'false'),
                                  ('deploy/compose/minio-init.sh', 'true', 'false'),
                                  ('backend/Cargo.toml', 'false', 'false')):
            with self.subTest(path=path):
                out = self.run_script(stdin=path + '\n')
                self.assertEqual((out['minio'], out['minio-build']), (area, build))
        # A release candidate (--all) builds from source: operators fall back to it.
        self.assertEqual(self.run_script('--all')['minio-build'], 'true')

    def test_all_runs_everything(self):
        out = self.run_script('--all')
        for area in changes.AREAS:
            self.assertEqual(out[area], 'true')
        self.assertEqual(len(json.loads(out['images'])), 3)


def copied(dockerfile, context, text=None):
    """What a Dockerfile's COPY and ADD lines and RUN bind mounts take from
    the build context, as repository paths; a folder ends in '/'."""
    text = re.sub(r'\\\n', ' ', text if text is not None else (ROOT / dockerfile).read_text())
    found = set()

    def add(source):
        path = (ROOT / context / source).resolve().relative_to(ROOT).as_posix()
        found.add(path + '/' if (ROOT / path).is_dir() else path)

    for line in text.splitlines():
        words = line.split()
        if not words:
            continue
        if words[0].upper() == 'RUN':
            for mount in re.findall(r'--mount=(\S+)', line):
                options = dict(option.partition('=')[::2] for option in mount.split(','))
                if options.get('type') == 'bind' and 'from' not in options:
                    add(options.get('source', '.'))
            continue
        if words[0].upper() not in ('COPY', 'ADD'):
            continue
        args = words[1:]
        if any(arg.startswith('--from=') for arg in args):
            continue
        for source in [arg for arg in args if not arg.startswith('--')][:-1]:
            add(source)
    return found


def outermost(paths):
    """The paths without those inside a folder that is listed too."""
    return {path for path in paths
            if not any(other != path and other.endswith('/') and path.startswith(other)
                       for other in paths)}


class Images(unittest.TestCase):
    """An image builds when, and only when, something it is built from changes."""

    def test_every_image_dockerfile_exists(self):
        for image in changes.IMAGES:
            with self.subTest(image=image['name']):
                self.assertTrue((ROOT / image['file']).is_file())
                self.assertTrue((ROOT / image['context']).is_dir())

    def test_the_inputs_are_what_the_dockerfile_copies(self):
        # Beyond the COPY lines: the Dockerfile, the context's .dockerignore,
        # ci.yml's images job, and the check run on the media image.
        checks = {'media': {'backend/build/check-media-image.py'}}
        for image in changes.IMAGES:
            with self.subTest(image=image['name']):
                sources = copied(image['file'], image['context'])
                self.assertTrue(sources)
                context = '' if image['context'] == '.' else image['context'] + '/'
                ignore = context + '.dockerignore'
                if sources == {context}:
                    # COPY . . takes the whole context, its .dockerignore with it.
                    ignore = context
                self.assertEqual(set(image['inputs']),
                                 outermost(sources | {image['file'], ignore, changes.CI}
                                           | checks.get(image['name'], set())))

    def test_a_bind_mount_is_an_input(self):
        found = copied('backend/api/Dockerfile', '.', 'RUN --mount=type=bind,source=backend/build,target=/b \\\n'
                       '    --mount=type=cache,target=/c --mount=type=bind,from=x,source=/y,target=/z true\n')
        self.assertEqual(found, {'backend/build/'})

    def test_an_api_change_builds_only_the_api_image(self):
        self.assertEqual(changes.images(['backend/api/src/main.rs']), {'api'})
        self.assertEqual(changes.images(['backend/media/src/main.rs']), {'media'})
        self.assertEqual(changes.images(['backend/shared/src/lib.rs', 'backend/Cargo.lock']),
                         {'api', 'media'})
        self.assertEqual(changes.images(['backend/shared/data/emoji-18.0.json']), {'api', 'media'})
        self.assertEqual(changes.images(['backend/build/check-media-image.py']), {'media'})
        self.assertEqual(changes.images(['.github/workflows/ci.yml']), {'api', 'media', 'web'})
        self.assertEqual(changes.images(['backend/api/README.md', 'deploy/backup.sh']), {'api'})

    def test_a_backend_change_outside_the_images_builds_none(self):
        self.assertEqual(changes.images(['backend/build/run-native-ci.sh']), set())


def areas_step():
    return next(step for step in workflow_data.load('ci.yml')['jobs']['changes']['steps']
                if step.get('name') == 'Areas')


def run_areas(env, diff=()):
    """ci.yml's Areas step, run with a fake git that answers `diff` with
    these paths: its outputs, and the git commands it ran."""
    with tempfile.TemporaryDirectory() as tmp:
        output, log = Path(tmp) / 'output', Path(tmp) / 'git.log'
        git = Path(tmp) / 'git'
        git.write_text('#!/bin/sh\necho "$*" >> "$GIT_LOG"\n'
                       'case "$*" in *" diff "*) printf "%s" "$FAKE_DIFF" ;; esac\n')
        git.chmod(0o755)
        full = {**os.environ, 'EVENT': 'pull_request', 'CANDIDATE': '', 'REUSED_RUN': '', 'BEFORE': '',
                'GITHUB_REF': 'refs/heads/v0.14',
                **env, 'GITHUB_OUTPUT': str(output), 'GITHUB_STEP_SUMMARY': os.devnull,
                'GIT_LOG': str(log), 'FAKE_DIFF': ''.join(path + '\n' for path in diff),
                'PATH': f'{tmp}{os.pathsep}{os.environ["PATH"]}'}
        subprocess.run(['bash', '-c', areas_step()['run']], cwd=ROOT, env=full, check=True,
                       capture_output=True)
        out = dict(line.split('=', 1) for line in output.read_text().splitlines())
        commands = log.read_text().splitlines() if log.exists() else []
    return {area for area in changes.AREAS if out[area] == 'true'}, commands


class Workflow(unittest.TestCase):
    """ci.yml uses the script and gates on the one check ci-ok."""

    def setUp(self):
        self.ci = workflow_data.load('ci.yml')

    def test_pull_requests_and_pushes_are_not_path_filtered(self):
        for event in ('pull_request', 'push', 'merge_group'):
            with self.subTest(event=event):
                self.assertFalse({'paths', 'paths-ignore'} & set(self.ci['on'][event] or {}))

    def test_a_pull_request_runs_the_areas_of_its_diff(self):
        ran, commands = run_areas({}, ['web/src/main.tsx'])
        self.assertEqual(ran, {'web'})
        self.assertIn('-c core.quotePath=false diff --no-renames --name-only HEAD^1 HEAD', commands)

    def test_a_push_runs_the_areas_since_its_starting_point(self):
        # GEL-360: a merge into a version branch no longer runs every area;
        # GEL-363: it starts at the last commit whose push run passed.
        self.assertEqual(areas_step()['env']['BEFORE'], '${{ steps.reuse.outputs.since || github.event.before '
                                                        '|| github.event.merge_group.base_sha }}')
        before = 'a' * 40
        ran, commands = run_areas({'EVENT': 'push', 'BEFORE': before}, ['backend/api/src/main.rs'])
        self.assertEqual(ran, {'backend', 'deploy'})
        self.assertIn(f'-c core.quotePath=false diff --no-renames --name-only {before} HEAD', commands)
        self.assertEqual(run_areas({'EVENT': 'push', 'BEFORE': '0' * 40}, ['docs/x.md'])[0], set(changes.AREAS))
        self.assertEqual(run_areas({'EVENT': 'push', 'REUSED_RUN': '42', 'BEFORE': before},
                                   ['backend/api/src/main.rs'])[0], set())

    def test_a_release_candidate_runs_every_area_main_reuses(self):
        """A release pull request whose only change is its notes: main
        reuses its run only with every area."""
        tested_tree = importlib.util.spec_from_file_location(
            'tested_tree', ROOT / '.github/workflows/scripts/tested_tree.py')
        reuse = importlib.util.module_from_spec(tested_tree)
        tested_tree.loader.exec_module(reuse)
        for candidate, expected in (('true', set(changes.AREAS)), ('false', set())):
            with self.subTest(candidate=candidate):
                ran, _ = run_areas({'CANDIDATE': candidate}, ['.github/release-notes/v9.9.9.md'])
                self.assertEqual(ran, expected)
                if candidate == 'true':
                    self.assertTrue(reuse.stands_for({'schema': 1, 'tree': 't', 'areas': sorted(ran)}, 't'))

    def test_every_area_has_a_gated_job(self):
        conditions = [str(job.get('if', '')) for job in self.ci['jobs'].values()]
        for area in changes.AREAS:
            with self.subTest(area=area):
                self.assertTrue([c for c in conditions if f"needs.changes.outputs.{area} == 'true'" in c])

    def test_ci_ok_needs_every_job(self):
        jobs = self.ci['jobs']
        self.assertEqual(set(jobs['ci-ok']['needs']), set(jobs) - {'ci-ok'})
        self.assertEqual(jobs['ci-ok']['if'], 'always()')

    def ci_ok(self, outputs, results=('success', 'skipped')):
        """ci-ok's check, run with these `needs` results and changes outputs:
        its exit code and what it printed."""
        step = next(step for step in self.ci['jobs']['ci-ok']['steps']
                    if step.get('name') == 'Every job that ran passed')
        areas = re.sub(r'\$\{\{ needs\.changes\.outputs\.([\w-]+) \}\}',
                       lambda match: outputs.get(match.group(1), ''), step['env']['AREAS'])
        result = subprocess.run(['bash', '-c', step['run']], capture_output=True, text=True,
                                env={**os.environ, 'RESULTS': json.dumps(list(results)), 'AREAS': areas})
        return result.returncode, result.stdout

    def test_every_area_has_an_output_and_a_check_in_ci_ok(self):
        # GEL-397: actionlint cannot see the names a run: step writes.
        jobs = self.ci['jobs']
        step = next(step for step in jobs['ci-ok']['steps'] if step.get('name') == 'Every job that ran passed')
        checked = re.findall(r'([\w-]+)=\$\{\{ needs\.changes\.outputs\.([\w-]+) \}\}', step['env']['AREAS'])
        self.assertEqual(sorted(checked), sorted((area, area) for area in changes.AREAS))
        for area in changes.AREAS:
            with self.subTest(area=area):
                self.assertEqual(jobs['changes']['outputs'][area], f'${{{{ steps.areas.outputs.{area} }}}}')
        written = dict(line.split('=', 1) for line in changes.outputs(set(), set()))
        self.assertLessEqual(set(changes.AREAS), set(written))

    def test_ci_ok_fails_on_an_area_output_that_is_not_true_or_false(self):
        good = {area: 'true' if area == 'web' else 'false' for area in changes.AREAS}
        self.assertEqual(self.ci_ok(good)[0], 0)
        for broken in ('', 'ture', 'True'):
            with self.subTest(value=broken):
                code, printed = self.ci_ok({**good, 'deploy': broken})
                self.assertNotEqual(code, 0)
                self.assertIn('output deploy', printed)
        self.assertNotEqual(self.ci_ok(good, ('success', 'failure'))[0], 0)

    def test_every_deployment_test_suite_runs(self):
        scripts = [script for job_id, _, script in workflow_data.scripts(self.ci) if job_id == 'deploy']
        folders = {suite.parent.relative_to(ROOT).as_posix() for suite in (ROOT / 'deploy').glob('**/tests/test_*.py')}
        for folder in sorted(folders):
            with self.subTest(folder=folder):
                self.assertTrue([s for s in scripts if f'python3 -m unittest discover -s {folder} ' in s])

    def test_a_push_to_main_that_reuses_no_run_checks_every_area(self):
        """Release merges ran every area only because of their release notes;
        main checks all of them whenever it cannot reuse a run (GEL-390)."""
        for ref, expected in (('refs/heads/main', set(changes.AREAS)), ('refs/heads/v0.14', {'web'})):
            with self.subTest(ref=ref):
                ran, _ = run_areas({'EVENT': 'push', 'GITHUB_REF': ref, 'BEFORE': 'a' * 40},
                                   ['web/src/main.tsx'])
                self.assertEqual(ran, expected)

    def test_moved_and_non_ascii_paths_start_their_areas(self):
        """The pull request's diff command, run on a scratch repository: a
        move names its source too, and a non-ASCII path stays unquoted."""
        command = re.search(r'^\s*(git .*diff .*HEAD\^1 HEAD) \|', areas_step()['run'], re.M).group(1)
        # Git's defaults as on a runner, not this machine's settings.
        env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
        with tempfile.TemporaryDirectory() as tmp:
            def git(*args):
                subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args],
                               cwd=tmp, env=env, check=True, capture_output=True)
            git('init', '-q')
            (Path(tmp) / 'backend').mkdir()
            (Path(tmp) / 'docs').mkdir()
            (Path(tmp) / 'backend/helper.rs').write_text('fn helper() {}\n' * 20)
            git('add', '.')
            git('commit', '-q', '-m', 'base')
            git('mv', 'backend/helper.rs', 'docs/helper.rs')
            (Path(tmp) / 'web/public/audio').mkdir(parents=True)
            (Path(tmp) / 'web/public/audio/Glöckchen.txt').write_text('x\n')
            git('add', '.')
            git('commit', '-q', '-m', 'change')
            paths = subprocess.run(command, shell=True, cwd=tmp, env=env, check=True,
                                   capture_output=True, text=True).stdout.splitlines()
        self.assertEqual(changed_areas(*paths), {'backend', 'deploy', 'web'})


if __name__ == '__main__':
    unittest.main()
