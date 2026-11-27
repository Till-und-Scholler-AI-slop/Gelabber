"""The Claude Code Stop hook that runs fast checks (.claude/hooks/fast_checks.py)."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from prerequisites import missing

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
    def names(self, *paths, root=ROOT):
        return [name for name, _, _ in hook.checks(root, list(paths))]

    def web_root(self):
        """A root whose web/ has dependencies, as far as the hook looks: it
        only asks whether web/node_modules is a directory."""
        shutil.which('node') or missing('node')
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        (Path(tmp.name) / 'web/node_modules').mkdir(parents=True)
        return Path(tmp.name)

    def test_nothing_changed_runs_nothing(self):
        self.assertEqual(self.names(), [])
        self.assertEqual(self.names('README.md', 'docs/design-qa.md'), [])

    def test_workflows_run_the_pin_test(self):
        self.assertEqual(self.names('.github/workflows/ci.yml'), ['workflows: action pins'])

    def rustfmt(self, installed):
        """The hook's own look for cargo fmt, answered by the test."""
        which = mock.patch.object(hook.shutil, 'which', return_value='/usr/bin/cargo')
        run = mock.patch.object(hook, 'run', return_value=(0 if installed else 1, ''))
        return which, run

    def test_a_rust_change_formats_its_own_workspace(self):
        which, run = self.rustfmt(installed=True)
        with which, run:
            self.assertEqual(self.names('backend/api/src/main.rs'), ['backend: cargo fmt'])
            self.assertEqual(self.names('desktop/ui/src/main.rs'), ['desktop: cargo fmt'])
        which, run = self.rustfmt(installed=False)
        with which, run:
            self.assertEqual(self.names('backend/api/src/main.rs'), [])

    def test_web_changes_pick_their_checks(self):
        root = self.web_root()
        self.assertEqual(self.names('web/src/main.tsx', root=root), ['web: eslint', 'web: tsc'])
        self.assertEqual(self.names('web/scripts/checks.mjs', root=root), ['web: eslint'])
        self.assertEqual(self.names('desktop/notes.tsx', root=root), [])

    def test_without_web_dependencies_there_are_no_web_checks(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        (Path(tmp.name) / 'web/src').mkdir(parents=True)
        self.assertEqual(self.names('web/src/main.tsx', root=Path(tmp.name)), [])

    def test_web_eslint_gets_paths_relative_to_web(self):
        root = self.web_root()
        found = {name: (args, cwd) for name, args, cwd in hook.checks(root, ['web/src/main.tsx'])}
        self.assertIn('web: eslint', found)
        eslint, cwd = found['web: eslint']
        self.assertEqual((eslint[-1], cwd), ('src/main.tsx', root / 'web'))


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
            self.assertIn('unpinned', json.loads(result.stdout)['systemMessage'])

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


if __name__ == '__main__':
    unittest.main()
