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

    def test_a_workflow_change_starts_every_area(self):
        self.assertEqual(changes.areas(['.github/workflows/ci.yml']), set(changes.AREAS))
        self.assertEqual(changes.areas(['.github/dependabot.yml']), set(changes.AREAS))

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
        self.assertEqual([image['name'] for image in json.loads(out['images'])], ['api', 'media'])

    def test_only_the_web_image_for_a_web_change(self):
        out = self.run_script(stdin='web/src/main.tsx\n')
        self.assertEqual(json.loads(out['images']), [
            {'name': 'web', 'context': 'web', 'file': 'web/Dockerfile'}])

    def test_no_images_is_an_empty_list(self):
        self.assertEqual(self.run_script(stdin='docs/x.md\n')['images'], '[]')

    def test_all_runs_everything(self):
        out = self.run_script('--all')
        for area in changes.AREAS:
            self.assertEqual(out[area], 'true')
        self.assertEqual(len(json.loads(out['images'])), 3)

    def test_every_image_dockerfile_exists(self):
        for built in changes.IMAGES.values():
            for image in built:
                with self.subTest(image=image['name']):
                    self.assertTrue((ROOT / image['file']).is_file())
                    self.assertTrue((ROOT / image['context']).is_dir())


class Workflow(unittest.TestCase):
    """ci.yml uses the script and gates on the one check ci-ok."""

    def setUp(self):
        self.ci = (ROOT / '.github/workflows/ci.yml').read_text()

    def test_pull_requests_and_pushes_are_not_path_filtered(self):
        triggers = self.ci.split('\non:\n', 1)[1].split('\nconcurrency:', 1)[0]
        self.assertNotIn('paths:', triggers)

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
