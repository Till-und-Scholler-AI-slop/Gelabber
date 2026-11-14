"""The Claude Code hook that enforces the branch naming rule (.claude/hooks/branch_name.py)."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_changes import changed_areas

ROOT = Path(__file__).resolve().parents[3]
HOOK = ROOT / '.claude/hooks/branch_name.py'


def load():
    spec = importlib.util.spec_from_file_location('branch_name', HOOK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


hook = load()


class Names(unittest.TestCase):
    def test_rule_names_pass(self):
        for name in ('fix/123-reconnect-after-sleep', 'feature/124-message-search', 'docs/9-readme',
                     'ci/577-branch-name-hook', 'refactor/1-x', 'chore/2-deps', 'release/v0.10.2',
                     'hotfix/v0.10.1-login-loop', 'sync/main-into-v0.10', 'v0.11', 'main',
                     'dependabot/cargo/backend/tokio-1.54.1'):
            with self.subTest(name=name):
                self.assertTrue(hook.NAME.match(name))

    def test_other_names_fail(self):
        for name in ('claude/project-thread-fildot', 'cursor/dependabot-improvements-0658', 'fix/reconnect',
                     'fix/123_Reconnect', 'feat/1-x', 'Fix/1-x', 'release/0.10.2', 'hotfix/v0.10.1',
                     'my-branch'):
            with self.subTest(name=name):
                self.assertFalse(hook.NAME.match(name))


class Commands(unittest.TestCase):
    def test_blocks_pushing_a_bad_name(self):
        for command in ('git push -u origin claude/x', 'git push origin claude/x:claude/x',
                        'cd repo && git push origin HEAD:refs/heads/claude/x',
                        'git -C repo push --force-with-lease origin +claude/x',
                        'git fetch origin && git push origin fix/1-x:claude/x'):
            with self.subTest(command=command):
                self.assertEqual(hook.violations(command), ['claude/x'])

    def test_lets_other_git_calls_through(self):
        for command in ('git push -u origin fix/1-x', 'git push origin claude/x:fix/1-x',
                        'git push origin --delete claude/x', 'git push origin :claude/x',
                        'git push --tags', 'git log --oneline', 'echo "git push origin claude/x"',
                        'git status; ls', "git commit -F - <<'EOF'\nmention git push origin claude/x\nEOF\n",
                        'git push "unclosed'):
            with self.subTest(command=command):
                self.assertEqual(hook.violations(command), [])

    def test_local_branches_stay_free(self):
        # The rule is about branches on origin, e.g. a PR fetched for review.
        for command in ('git checkout -B pr-123 FETCH_HEAD', 'git switch -c claude/x',
                        'git branch claude/x', 'git branch -m claude/x'):
            with self.subTest(command=command):
                self.assertEqual(hook.violations(command), [])

    def test_a_bare_push_checks_the_current_branch(self):
        with tempfile.TemporaryDirectory() as repo:
            subprocess.run(['git', 'init', '-q', '-b', 'claude/x', repo], check=True)
            subprocess.run(['git', '-C', repo, '-c', 'user.name=t', '-c', 'user.email=t@t',
                            'commit', '-q', '--allow-empty', '-m', 'x'], check=True)
            self.assertEqual(hook.violations(f'git -C {repo} push'), ['claude/x'])
            self.assertEqual(hook.violations(f'git -C {repo} push -u origin HEAD'), ['claude/x'])


class Hook(unittest.TestCase):
    def run_hook(self, command):
        payload = json.dumps({'tool_name': 'Bash', 'tool_input': {'command': command}})
        return subprocess.run([sys.executable, str(HOOK)], input=payload, capture_output=True, text=True)

    def test_exit_code_2_blocks_with_the_rule(self):
        result = self.run_hook('git push -u origin claude/x')
        self.assertEqual(result.returncode, 2)
        self.assertIn('AGENTS.md', result.stderr)

    def test_exit_code_0_lets_it_run(self):
        self.assertEqual(self.run_hook('git push -u origin fix/1-x').returncode, 0)

    def test_settings_run_the_hook_for_bash(self):
        settings = json.loads((ROOT / '.claude/settings.json').read_text(encoding='utf-8'))
        commands = [hook['command'] for entry in settings['hooks']['PreToolUse']
                    if entry['matcher'] == 'Bash' for hook in entry['hooks']]
        self.assertIn('python3 "$CLAUDE_PROJECT_DIR/.claude/hooks/branch_name.py"', commands)

    def test_a_change_to_the_hooks_starts_ci(self):
        self.assertIn('deploy', changed_areas('.claude/hooks/branch_name.py'))
        self.assertIn('deploy', changed_areas('.claude/settings.json'))


if __name__ == '__main__':
    unittest.main()
