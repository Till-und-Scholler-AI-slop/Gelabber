"""The Claude Code Stop hook that runs fast checks (.claude/hooks/fast_checks.py)."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_changes import changed_areas

ROOT = Path(__file__).resolve().parents[3]
HOOK = ROOT / '.claude/hooks/fast_checks.py'


def load():
    spec = importlib.util.spec_from_file_location('fast_checks', HOOK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


hook = load()


def git(repo, *args):
    subprocess.run(['git', '-C', str(repo), '-c', 'user.name=t', '-c', 'user.email=t@t', *args],
                   check=True, capture_output=True)


class ChangedFiles(unittest.TestCase):
    def test_finds_edited_untracked_and_unpushed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            git(repo, 'init', '-q')
            for name in ('kept.txt', 'edited.txt', 'committed.txt'):
                (repo / name).write_text('a')
            git(repo, 'add', 'kept.txt', 'edited.txt')
            git(repo, 'commit', '-q', '-m', 'base')
            git(repo, 'update-ref', 'refs/remotes/origin/main', 'HEAD')
            git(repo, 'add', 'committed.txt')
            git(repo, 'commit', '-q', '-m', 'local')
            (repo / 'edited.txt').write_text('b')
            (repo / 'new.txt').write_text('c')
            self.assertEqual(hook.changed_files(repo), ['committed.txt', 'edited.txt', 'new.txt'])


class Checks(unittest.TestCase):
    def names(self, *paths):
        return [name for name, _, _ in hook.checks(ROOT, list(paths))]

    def test_nothing_changed_runs_nothing(self):
        self.assertEqual(self.names(), [])
        self.assertEqual(self.names('README.md', 'docs/design-qa.md'), [])

    def test_workflows_run_the_pin_test(self):
        self.assertEqual(self.names('.github/workflows/ci.yml'), ['workflows: action pins'])

    def test_rust_and_web_changes_pick_their_checks(self):
        names = self.names('backend/api/src/main.rs', 'web/src/main.tsx')
        if any(name.endswith('cargo fmt') for name in names):
            self.assertIn('backend: cargo fmt', names)
            self.assertNotIn('desktop: cargo fmt', names)
        if (ROOT / 'web/node_modules').is_dir():
            self.assertIn('web: eslint', names)
            self.assertIn('web: tsc', names)

    def test_web_eslint_gets_paths_relative_to_web(self):
        if not (ROOT / 'web/node_modules').is_dir():
            self.skipTest('web dependencies are not installed')
        eslint = next(args for name, args, _ in hook.checks(ROOT, ['web/src/main.tsx']) if name == 'web: eslint')
        self.assertEqual(eslint[-1], 'src/main.tsx')


class Hook(unittest.TestCase):
    def test_a_clean_tree_lets_claude_stop(self):
        with tempfile.TemporaryDirectory() as tmp:
            git(tmp, 'init', '-q')
            git(tmp, 'commit', '-q', '--allow-empty', '-m', 'x')
            git(tmp, 'update-ref', 'refs/remotes/origin/main', 'HEAD')
            result = subprocess.run([sys.executable, str(HOOK)], input=json.dumps({'stop_hook_active': False}),
                                    capture_output=True, text=True, env={'CLAUDE_PROJECT_DIR': tmp, 'PATH': '/usr/bin:/bin'})
            self.assertEqual(result.returncode, 0, result.stderr)

    def failing_repo(self, tmp):
        repo = Path(tmp)
        git(repo, 'init', '-q')
        git(repo, 'commit', '-q', '--allow-empty', '-m', 'x')
        git(repo, 'update-ref', 'refs/remotes/origin/main', 'HEAD')
        tests = repo / 'deploy/compose/tests'
        tests.mkdir(parents=True)
        (tests / 'test_workflow_pins.py').write_text(
            'import unittest\nclass T(unittest.TestCase):\n    def test(self):\n        self.fail("unpinned")\n')
        (repo / '.github/workflows').mkdir(parents=True)
        (repo / '.github/workflows/ci.yml').write_text('on: push\n')

    def run_hook(self, tmp, data):
        return subprocess.run([sys.executable, str(HOOK)], input=json.dumps(data), capture_output=True, text=True,
                              env={'CLAUDE_PROJECT_DIR': tmp, 'PATH': '/usr/bin:/bin'})

    def test_a_second_stop_is_not_blocked_again(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.failing_repo(tmp)
            result = self.run_hook(tmp, {'stop_hook_active': True})
            self.assertEqual(result.returncode, 0)
            self.assertIn('unpinned', result.stderr)

    def test_a_failing_check_blocks_with_its_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            git(repo, 'init', '-q')
            git(repo, 'commit', '-q', '--allow-empty', '-m', 'x')
            git(repo, 'update-ref', 'refs/remotes/origin/main', 'HEAD')
            tests = repo / 'deploy/compose/tests'
            tests.mkdir(parents=True)
            (tests / 'test_workflow_pins.py').write_text(
                'import unittest\nclass T(unittest.TestCase):\n    def test(self):\n        self.fail("unpinned")\n')
            (repo / '.github/workflows').mkdir(parents=True)
            (repo / '.github/workflows/ci.yml').write_text('on: push\n')
            result = subprocess.run([sys.executable, str(HOOK)], input='{}', capture_output=True, text=True,
                                    env={'CLAUDE_PROJECT_DIR': tmp, 'PATH': '/usr/bin:/bin'})
            self.assertEqual(result.returncode, 2)
            self.assertIn('workflows: action pins failed', result.stderr)
            self.assertIn('unpinned', result.stderr)

    def test_settings_run_the_hook_on_stop(self):
        settings = json.loads((ROOT / '.claude/settings.json').read_text(encoding='utf-8'))
        commands = [hook['command'] for entry in settings['hooks']['Stop'] for hook in entry['hooks']]
        self.assertIn('python3 "$CLAUDE_PROJECT_DIR/.claude/hooks/fast_checks.py"', commands)

    def test_a_change_to_the_hook_starts_ci(self):
        self.assertIn('deploy', changed_areas('.claude/hooks/fast_checks.py'))


if __name__ == '__main__':
    unittest.main()
