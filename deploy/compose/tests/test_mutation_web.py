"""Web mutation testing picks the changed lines, reports what no test
detected, and keeps one issue current (.github/workflows/scripts/mutation_web.py,
mutation-web.yml, GEL-388).

The Stryker report below has the shape Stryker 10.1.1 writes
(`reports/mutation/mutation.json`, schema 1.0).
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from workflow_data import load, writes

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/mutation_web.py'


def load_script():
    spec = importlib.util.spec_from_file_location('mutation_web', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


mutation_web = load_script()
SHA = 'a' * 40


def git(repo, *args):
    return subprocess.run(('git', '-C', repo, *args), check=True, capture_output=True, text=True).stdout


def diff_of(before, after):
    """`git diff -U0` between two trees, each {path: text}, as the plan reads it."""
    with tempfile.TemporaryDirectory() as repo:
        git(repo, 'init', '-q')
        for tree in (before, after):
            for path in Path(repo).glob('web'):
                subprocess.run(('rm', '-r', str(path)), check=True)
            for path, text in tree.items():
                (Path(repo) / path).parent.mkdir(parents=True, exist_ok=True)
                (Path(repo) / path).write_text(text)
            git(repo, 'add', '-A')
            git(repo, '-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '--allow-empty', '-m', 'x')
        return git(repo, 'diff', '--no-color', '--no-ext-diff', '--no-renames', '-U0', 'HEAD~1', 'HEAD', '--', 'web/src')


def lines(count, prefix='line'):
    return ''.join(f'{prefix} {n}\n' for n in range(1, count + 1))


def mutant(status, line, mutator='ConditionalExpression', replacement='false', end=None):
    return {'id': str(line), 'mutatorName': mutator, 'replacement': replacement, 'status': status,
            'static': False, 'location': {'start': {'line': line, 'column': 1},
                                          'end': {'line': end or line, 'column': 6}}}


REPORT = {'schemaVersion': '1.0', 'thresholds': {'high': 80, 'low': 60, 'break': None}, 'files': {
    'src/b.ts': {'language': 'typescript', 'source': 'a | b\nfine\n', 'mutants': [
        mutant('Survived', 1, 'LogicalOperator', 'a & b'), mutant('Killed', 2)]},
    'src/a.ts': {'language': 'typescript', 'source': 'first\nsecond\nthird\n', 'mutants': [
        mutant('Timeout', 1), mutant('NoCoverage', 3, 'StringLiteral', '""'), mutant('Survived', 2),
        mutant('Killed', 1), mutant('CompileError', 2)]},
}}
PLAN = {'files': [{'path': 'web/src/a.ts', 'lines': 3, 'ranges': [[1, 3]]},
                  {'path': 'web/src/b.ts', 'lines': 2, 'ranges': [[1, 2]]}],
        'skipped': [], 'lines': 5, 'limits': {'files': 60, 'lines': 1500},
        'base': 'v0.14.0', 'sha': SHA, 'label': '`v0.15`'}


class Target(unittest.TestCase):
    def test_the_development_line_is_the_highest_version_branch(self):
        ls_remote = ''.join(f'{"1" * 40}\trefs/heads/{name}\n'
                            for name in ('v0.9', 'v0.15', 'v0.14', 'v0.14-old', 'vnext', 'main'))
        self.assertEqual(mutation_web.newest_version_branch(ls_remote), 'v0.15')
        with self.assertRaises(ValueError):
            mutation_web.newest_version_branch(f'{"1" * 40}\trefs/heads/main\n')

    def test_a_pull_request_tests_its_merge_commit_and_a_manual_run_its_branch(self):
        lister = mock.Mock(return_value=f'{"1" * 40}\trefs/heads/v0.15\n')
        self.assertIsNone(mutation_web.choose('pull_request', '', lister))
        self.assertEqual(mutation_web.choose('workflow_dispatch', 'fix/gel-1-x', lister), 'fix/gel-1-x')
        lister.assert_not_called()
        self.assertEqual(mutation_web.choose('workflow_dispatch', '', lister), 'v0.15')
        self.assertEqual(mutation_web.choose('schedule', '', lister), 'v0.15')
        for bad in ('-x', 'a..b', 'a b', 'refs/heads/$(x)'):
            with self.subTest(ref=bad), self.assertRaises(ValueError):
                mutation_web.choose('workflow_dispatch', bad, lister)


class Plan(unittest.TestCase):
    def test_only_added_or_changed_source_lines_are_mutated(self):
        before = {'web/src/a.ts': lines(10), 'web/src/gone.ts': lines(3), 'web/src/cut.ts': lines(5),
                  'web/src/a.test.ts': lines(2), 'web/src/i18n/catalogs/chat.ts': lines(2),
                  'web/src/style.css': lines(2), 'web/src/types.d.ts': lines(2)}
        after = {
            # Line 2 changed, lines 5-6 added after 4, line 9 deleted.
            'web/src/a.ts': 'line 1\nchanged\nline 3\nline 4\nnew\nnew\nline 5\nline 6\nline 7\nline 8\nline 10\n',
            'web/src/new.tsx': lines(3),
            # Only deletions: nothing to mutate.
            'web/src/cut.ts': 'line 1\nline 5\n',
            'web/src/a.test.ts': lines(2, 'x'), 'web/src/i18n/catalogs/chat.ts': lines(2, 'x'),
            'web/src/style.css': lines(2, 'x'), 'web/src/types.d.ts': lines(2, 'x'),
        }
        result = mutation_web.plan_from_diff(diff_of(before, after))
        self.assertEqual(result['files'], [{'path': 'web/src/a.ts', 'lines': 3, 'ranges': [(2, 2), (5, 6)]},
                                           {'path': 'web/src/new.tsx', 'lines': 3, 'ranges': [(1, 3)]}])
        self.assertEqual(result['skipped'], [])
        self.assertEqual(result['lines'], 6)
        self.assertEqual(mutation_web.patterns(result), 'src/a.ts:2-2,src/a.ts:5-6,src/new.tsx:1-3')

    def test_a_large_diff_is_cut_and_says_what_it_left_out(self):
        after = {f'web/src/f{n}.ts': lines(n) for n in (4, 5, 6, 7)}
        diff = diff_of({}, after)
        by_files = mutation_web.plan_from_diff(diff, max_files=2, max_lines=100)
        self.assertEqual([f['path'] for f in by_files['files']], ['web/src/f4.ts', 'web/src/f5.ts'])
        self.assertEqual(by_files['skipped'], [
            {'path': 'web/src/f6.ts', 'lines': 6, 'reason': 'over the limit of 2 files'},
            {'path': 'web/src/f7.ts', 'lines': 7, 'reason': 'over the limit of 2 files'}])
        by_lines = mutation_web.plan_from_diff(diff, max_files=10, max_lines=10)
        self.assertEqual([f['path'] for f in by_lines['files']], ['web/src/f4.ts', 'web/src/f5.ts'])
        self.assertEqual([s['reason'] for s in by_lines['skipped']], ['over the limit of 10 changed lines'] * 2)
        text = mutation_web.report_text({**by_lines, 'base': 'v0.14.0', 'sha': SHA, 'label': '`v0.15`'},
                                        {}, [], 10000)
        self.assertIn('**Cut:** 2 files past the limits (10 files, 10 lines) were not mutated: '
                      '`web/src/f6.ts` (6 lines), `web/src/f7.ts` (7 lines).', text)


class Summary(unittest.TestCase):
    def test_lists_every_mutant_no_test_detected_by_file_and_line(self):
        counts, found = mutation_web.undetected(REPORT)
        self.assertEqual(counts, {'Survived': 2, 'Killed': 2, 'Timeout': 1, 'NoCoverage': 1, 'CompileError': 1})
        self.assertEqual([(f, line, status) for f, line, _, status, *_ in found],
                         [('src/a.ts', 2, 'Survived'), ('src/a.ts', 3, 'NoCoverage'), ('src/b.ts', 1, 'Survived')])
        text = mutation_web.report_text(PLAN, counts, found, 100000)
        self.assertIn('| 2 | 1 | 2 | 1 | 1 | 50.0 % |', text)
        self.assertIn('| `web/src/a.ts` | 3 | no test reaches it | StringLiteral | <code>third</code> '
                      '| <code>&quot;&quot;</code> |', text)
        # A | in code would end the table cell.
        self.assertIn('| `web/src/b.ts` | 1 | survived | LogicalOperator | <code>a &#124; b</code> '
                      '| <code>a &amp; b</code> |', text)
        for limit in ('equivalent mutants', 'pins today', 'gates nothing', 'timeout'):
            self.assertIn(limit, text)

    def test_a_long_list_is_cut_to_fit_and_points_to_the_artifact(self):
        many = {'files': {'src/a.ts': {'language': 'typescript', 'source': lines(200),
                                       'mutants': [mutant('Survived', n) for n in range(1, 201)]}}}
        counts, found = mutation_web.undetected(many)
        text = mutation_web.report_text(PLAN, counts, found, 8000)
        self.assertLessEqual(len(text), 8000)
        shown = text.count('| survived |')
        self.assertGreater(shown, 0)
        self.assertIn(f'{200 - shown} more are in the run\'s artifact `mutation-web`.', text)

    def test_writes_the_summary_and_what_the_issue_needs(self):
        with tempfile.TemporaryDirectory() as tmp, contextlib.chdir(tmp):
            Path('report.json').write_text(json.dumps(REPORT))
            Path('plan.json').write_text(json.dumps(PLAN))
            env = {'GITHUB_STEP_SUMMARY': f'{tmp}/summary.md', 'GITHUB_OUTPUT': f'{tmp}/output'}
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('report.json', 'plan.json')
            self.assertIn('### 3 mutants no test detected', Path('summary.md').read_text())
            self.assertEqual(Path('output').read_text(), 'undetected=3\n')
            info = json.loads(Path('issue.json').read_text())
            self.assertEqual((info['count'], info['sha']), (3, SHA))
            self.assertEqual(info['fingerprint'], mutation_web.fingerprint(mutation_web.undetected(REPORT)[1]))
            # Nothing changed: no report needed, nothing undetected.
            Path('empty.json').write_text(json.dumps({**PLAN, 'files': [], 'lines': 0}))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('missing.json', 'empty.json')
            self.assertEqual(json.loads(Path('issue.json').read_text())['count'], 0)
            # Lines to mutate but no report: Stryker did not finish.
            with mock.patch.dict(os.environ, env), self.assertRaises(FileNotFoundError):
                mutation_web.summary('missing.json', 'plan.json')


INFO = {'count': 3, 'fingerprint': 'f' * 64, 'sha': SHA, 'body': 'the list'}
RUN = 'https://github.com/o/r/actions/runs/1'


class Issue(unittest.TestCase):
    def test_one_issue_is_opened_replaced_or_closed(self):
        decide = mutation_web.decide
        self.assertEqual(decide(None, {**INFO, 'count': 0}, RUN), [])
        self.assertEqual(decide((7, 'old'), {**INFO, 'count': 0}, RUN),
                         [('comment', 7, f'No mutant survives at {SHA[:12]}: {RUN}'), ('close', 7, None)])
        [(action, number, body)] = decide(None, INFO, RUN)
        self.assertEqual((action, number), ('create', None))
        self.assertIn('the list', body)
        self.assertIn('Linear', body)
        self.assertIn(f'<!-- mutation-web: {"f" * 64} -->', body)
        # The same survivors again: no edit, no comment.
        self.assertEqual(decide((7, body), INFO, RUN), [])
        changed = decide((7, body), {**INFO, 'fingerprint': 'e' * 64}, RUN)
        self.assertEqual([(a, n) for a, n, _ in changed], [('edit', 7), ('comment', 7)])
        self.assertIn('e' * 64, changed[0][2])

    def run_issue(self, existing, dry_run):
        calls = []

        def fake_run(*args, stdin=None):
            calls.append((args, stdin))
            if args[:3] == ('gh', 'issue', 'list'):
                return json.dumps(existing)
            return 'https://github.com/o/r/issues/9\n'
        env = {'GITHUB_REPOSITORY': 'o/r', 'GITHUB_SERVER_URL': 'https://github.com', 'GITHUB_RUN_ID': '1'}
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'issue.json').write_text(json.dumps(INFO))
            out = io.StringIO()
            with mock.patch.object(mutation_web, 'run', fake_run), mock.patch.dict(os.environ, env), \
                    contextlib.redirect_stdout(out):
                mutation_web.issue(f'{tmp}/issue.json', dry_run)
        return calls, out.getvalue()

    def test_a_scheduled_run_writes_through_gh(self):
        other = {'number': 3, 'title': f'{mutation_web.TITLE} (old)', 'body': ''}
        calls, _ = self.run_issue([other], dry_run=False)
        self.assertEqual(calls[0][0][:3], ('gh', 'issue', 'list'))
        self.assertIn(f'in:title "{mutation_web.TITLE}"', calls[0][0])
        [(args, stdin)] = calls[1:]
        self.assertEqual(args, ('gh', 'issue', 'create', '--repo', 'o/r', '--title', mutation_web.TITLE,
                                '--body-file', '-'))
        self.assertIn('the list', stdin)
        mine = {'number': 5, 'title': mutation_web.TITLE, 'body': 'older list'}
        calls, _ = self.run_issue([other, mine], dry_run=False)
        self.assertEqual([args[:4] for args, _ in calls[1:]],
                         [('gh', 'issue', 'edit', '5'), ('gh', 'issue', 'comment', '5')])

    def test_a_dry_run_only_reads(self):
        calls, out = self.run_issue([], dry_run=True)
        self.assertEqual([args[:3] for args, _ in calls], [('gh', 'issue', 'list')])
        self.assertIn('Would create an issue:', out)
        self.assertIn('the list', out)


class Workflow(unittest.TestCase):
    def test_only_the_scheduled_runs_report_job_writes_the_issue(self):
        workflow = load('mutation-web.yml')
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        jobs = workflow['jobs']
        self.assertEqual({job_id for job_id, job in jobs.items() if writes(job.get('permissions'))}, {'report'})
        self.assertEqual(writes(jobs['report']['permissions']), {'issues'})
        self.assertEqual(jobs['report']['if'], "github.event_name == 'schedule'")
        self.assertEqual(set(workflow['on']), {'workflow_dispatch', 'schedule', 'pull_request'})

    def test_a_change_to_the_workflow_or_its_files_tests_it_again(self):
        paths = load('mutation-web.yml')['on']['pull_request']['paths']
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(list(ROOT.glob(path.removesuffix('/**') + '/*') if path.endswith('/**')
                                     else [ROOT / path] if (ROOT / path).is_file() else []))
        self.assertIn('.github/workflows/scripts/mutation_web.py', paths)
        self.assertIn('.github/workflows/mutation-web.yml', paths)
        # Stryker's config and its pinned packages, a Dependabot update of them too.
        self.assertIn('web/scripts/mutation/**', paths)


if __name__ == '__main__':
    unittest.main()
