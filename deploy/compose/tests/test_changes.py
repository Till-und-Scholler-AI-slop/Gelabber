"""The areas ci.yml runs for a pull request (.github/workflows/scripts/changes.py)."""
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

import yaml

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

    def test_the_storage_keys_the_desktop_test_reads_concern_it_too(self):
        legacy = (ROOT / 'desktop/ui/src/platform/legacy.rs').read_text(encoding='utf-8')
        read = set(re.findall(r'include_str!\("(?:\.\./)+(web/[^"]+)"\)', legacy))
        self.assertTrue(read)
        for path in read:
            with self.subTest(path=path):
                self.assertEqual(changes.areas([path]), {'web', 'desktop'})

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
        jobs = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())['jobs']
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
        # The checks that read them: test_public_pages.py, test_update_signature.py,
        # this file (legacy.rs), generate-emoji-data.py --check, landingImage.test.mjs,
        # ci-fault-loop.checks.mjs (a step of e2e-extended.yml).
        for path, area in (('web/docker-entrypoint.d/50-gelabber-public-pages.sh', 'deploy'),
                           ('desktop/packaging/updater.pub', 'deploy'),
                           ('desktop/ui/src/platform/legacy.rs', 'deploy'),
                           ('desktop/ui/assets/emoji/emoji-data.json', 'web'),
                           ('.github/workflows/e2e-extended.yml', 'web'),
                           ('docs/design/v0.10/landing-preview-en.png', 'web')):
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
        self.assertEqual(self.run_script('--all')['minio-build'], 'false')

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


class Workflow(unittest.TestCase):
    """ci.yml uses the script and gates on the one check ci-ok."""

    def setUp(self):
        self.ci = (ROOT / '.github/workflows/ci.yml').read_text()

    def test_pull_requests_and_pushes_are_not_path_filtered(self):
        triggers = self.ci.split('\non:\n', 1)[1].split('\nconcurrency:', 1)[0]
        self.assertNotIn('paths:', triggers)

    def test_a_push_runs_the_areas_since_its_previous_head(self):
        # GEL-360: a merge into a version branch no longer runs every area.
        areas = self.ci.split('      - name: Areas\n', 1)[1].split('\n      - name:', 1)[0]
        self.assertIn('BEFORE: ${{ steps.reuse.outputs.since || github.event.before || github.event.merge_group.base_sha }}', areas)
        self.assertIn('diff --no-renames --name-only "$BEFORE" HEAD | python3 .github/workflows/scripts/changes.py', areas)
        self.assertIn('changes.py --all', areas)

    def test_every_area_has_a_gated_job(self):
        for area in changes.AREAS:
            with self.subTest(area=area):
                self.assertIn(f"needs.changes.outputs.{area} == 'true'", self.ci)

    def test_ci_ok_needs_every_job(self):
        jobs = [line[2:-1] for line in self.ci.split('\njobs:\n', 1)[1].splitlines()
                if line.startswith('  ') and not line.startswith('   ') and line.endswith(':')]
        block = self.ci.split('\n  ci-ok:\n', 1)[1]
        needs = block.split('needs: [', 1)[1].split(']', 1)[0]
        self.assertEqual(sorted(name.strip() for name in needs.split(',')),
                         sorted(job for job in jobs if job != 'ci-ok'))
        self.assertIn('if: always()', block)

    def test_every_deployment_test_suite_runs(self):
        for suite in sorted((ROOT / 'deploy').glob('**/tests/test_*.py')):
            folder = suite.parent.relative_to(ROOT).as_posix()
            with self.subTest(folder=folder):
                self.assertIn(f'python3 -m unittest discover -s {folder} ', self.ci)

    def test_a_release_candidate_runs_every_area_main_reuses(self):
        """The Areas step, run as for a release pull request whose only
        change is its notes: main reuses its run only with every area."""
        step = self.ci.split('      - name: Areas\n', 1)[1].split('\n\n', 1)[0]
        script = textwrap.dedent(step.split('        run: |\n', 1)[1])
        tested_tree = importlib.util.spec_from_file_location(
            'tested_tree', ROOT / '.github/workflows/scripts/tested_tree.py')
        reuse = importlib.util.module_from_spec(tested_tree)
        tested_tree.loader.exec_module(reuse)
        for candidate, expected in (('true', set(changes.AREAS)), ('false', set())):
            with self.subTest(candidate=candidate), tempfile.TemporaryDirectory() as tmp:
                output = Path(tmp) / 'output'
                env = {**os.environ, 'EVENT': 'pull_request', 'CANDIDATE': candidate,
                       'GITHUB_OUTPUT': str(output), 'GITHUB_STEP_SUMMARY': os.devnull}
                # The pull request's diff: the release notes alone.
                git = Path(tmp) / 'git'
                git.write_text('#!/bin/sh\necho .github/release-notes/v9.9.9.md\n')
                git.chmod(0o755)
                env['PATH'] = f'{tmp}{os.pathsep}{env["PATH"]}'
                subprocess.run(['bash', '-c', script], cwd=ROOT, env=env, check=True, capture_output=True)
                out = dict(line.split('=', 1) for line in output.read_text().splitlines())
                ran = {area for area in changes.AREAS if out[area] == 'true'}
                self.assertEqual(ran, expected)
                if candidate == 'true':
                    self.assertTrue(reuse.stands_for({'schema': 1, 'tree': 't', 'areas': sorted(ran)}, 't'))

    def test_a_push_to_main_that_reuses_no_run_checks_every_area(self):
        """Release merges ran every area only because of their release notes;
        main must check all of them whenever it cannot reuse a run."""
        step = self.ci.split('      - name: Areas\n', 1)[1].split('\n\n', 1)[0]
        script = textwrap.dedent(step.split('        run: |\n', 1)[1])
        for ref, expected in (('refs/heads/main', set(changes.AREAS)), ('refs/heads/v0.14', {'web'})):
            with self.subTest(ref=ref), tempfile.TemporaryDirectory() as tmp:
                output = Path(tmp) / 'output'
                git = Path(tmp) / 'git'
                git.write_text('#!/bin/sh\ncase "$*" in *" diff "*) echo web/src/main.tsx ;; esac\n')
                git.chmod(0o755)
                env = {**os.environ, 'EVENT': 'push', 'GITHUB_REF': ref, 'BEFORE': 'a' * 40,
                       'REUSED_RUN': '', 'CANDIDATE': '', 'GITHUB_OUTPUT': str(output),
                       'GITHUB_STEP_SUMMARY': os.devnull,
                       'PATH': f'{tmp}{os.pathsep}{os.environ["PATH"]}'}
                subprocess.run(['bash', '-c', script], cwd=ROOT, env=env, check=True, capture_output=True)
                out = dict(line.split('=', 1) for line in output.read_text().splitlines())
                self.assertEqual({area for area in changes.AREAS if out[area] == 'true'}, expected)

    def test_moved_and_non_ascii_paths_start_their_areas(self):
        """The pull request's diff command, run on a scratch repository: a
        move names its source too, and a non-ASCII path stays unquoted."""
        command = re.search(r'^\s*(git .*diff .*HEAD\^1 HEAD) \|', self.ci, re.M).group(1)
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
