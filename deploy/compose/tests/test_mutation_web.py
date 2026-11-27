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


def write_tree(root, tree):
    for path, text in tree.items():
        (Path(root) / path).parent.mkdir(parents=True, exist_ok=True)
        (Path(root) / path).write_text(text)


def planned(before, after, **limits):
    """plan() on a repository whose release v0.0.1 holds `before` and whose
    next commit holds `after` (each {path: text}): (plan.json, step outputs)."""
    with tempfile.TemporaryDirectory() as repo:
        git(repo, 'init', '-q')
        for index, tree in enumerate((before, after)):
            subprocess.run(('rm', '-rf', str(Path(repo) / 'web')), check=True)
            write_tree(repo, tree)
            git(repo, 'add', '-A')
            git(repo, '-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '--allow-empty', '-m', 'x')
            if index == 0:
                git(repo, 'tag', 'v0.0.1')
        sha = git(repo, 'rev-parse', 'HEAD').strip()
        env = {'GITHUB_OUTPUT': f'{repo}/.output', 'LABEL': '`v9.9`'}
        patches = [mock.patch.object(mutation_web, name.upper(), value) for name, value in limits.items()]
        with contextlib.chdir(repo), mock.patch.dict(os.environ, env), contextlib.ExitStack() as stack, \
                contextlib.redirect_stdout(io.StringIO()):
            for patch in patches:
                stack.enter_context(patch)
            mutation_web.plan('v0.0.1', sha)
        return json.loads(Path(repo, 'plan.json').read_text()), Path(repo, '.output').read_text()


def lines(count, prefix='line'):
    return ''.join(f'{prefix} {n}\n' for n in range(1, count + 1))


def mutant(status, line, mutator='ConditionalExpression', replacement='false'):
    return {'id': str(line), 'mutatorName': mutator, 'replacement': replacement, 'status': status,
            'static': False, 'location': {'start': {'line': line, 'column': 1}, 'end': {'line': line, 'column': 6}}}


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


def many(count, replacement='x'):
    return {'files': {'src/a.ts': {'language': 'typescript', 'source': lines(count),
                                   'mutants': [mutant('Survived', n, replacement=replacement)
                                               for n in range(1, count + 1)]}}}


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

    def test_a_failed_command_shows_its_error(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(subprocess.CalledProcessError):
            mutation_web.run('sh', '-c', 'echo "gh: Not Found (HTTP 404)" >&2; exit 1')
        self.assertIn('gh: Not Found (HTTP 404)', err.getvalue())


class Plan(unittest.TestCase):
    def test_only_added_or_changed_source_lines_are_mutated(self):
        before = {'web/src/a.ts': lines(10), 'web/src/gone.ts': lines(3), 'web/src/cut.ts': lines(5),
                  'web/src/a.test.ts': lines(2), 'web/src/i18n/catalogs/chat.ts': lines(2),
                  'web/src/style.css': lines(2), 'web/src/types.d.ts': lines(2), 'web/other.ts': lines(2)}
        after = {
            # Line 2 changed, lines 5-6 added after 4, line 9 deleted.
            'web/src/a.ts': 'line 1\nchanged\nline 3\nline 4\nnew\nnew\nline 5\nline 6\nline 7\nline 8\nline 10\n',
            'web/src/new.tsx': lines(3),
            # Only deletions: nothing to mutate.
            'web/src/cut.ts': 'line 1\nline 5\n',
            'web/src/a.test.ts': lines(2, 'x'), 'web/src/i18n/catalogs/chat.ts': lines(2, 'x'),
            'web/src/style.css': lines(2, 'x'), 'web/src/types.d.ts': lines(2, 'x'), 'web/other.ts': lines(2, 'x'),
        }
        result, outputs = planned(before, after)
        self.assertEqual(result['files'], [{'path': 'web/src/a.ts', 'lines': 3, 'ranges': [[2, 2], [5, 6]]},
                                           {'path': 'web/src/new.tsx', 'lines': 3, 'ranges': [[1, 3]]}])
        self.assertEqual((result['skipped'], result['lines'], result['base'], result['label']),
                         ([], 6, 'v0.0.1', '`v9.9`'))
        self.assertEqual(outputs, 'mutate=src/a.ts:2-2,src/a.ts:5-6,src/new.tsx:1-3\nfiles=2\n')

    def test_a_file_past_a_limit_is_left_out_and_smaller_ones_still_go_in(self):
        after = {'web/src/a.ts': lines(4), 'web/src/b.ts': lines(8), 'web/src/c.ts': lines(3)}
        by_lines, outputs = planned({}, after, max_lines=10)
        self.assertEqual([f['path'] for f in by_lines['files']], ['web/src/a.ts', 'web/src/c.ts'])
        self.assertEqual(by_lines['skipped'],
                         [{'path': 'web/src/b.ts', 'lines': 8, 'reason': 'over the limit of 10 changed lines'}])
        self.assertEqual(outputs, 'mutate=src/a.ts:1-4,src/c.ts:1-3\nfiles=2\n')
        by_files, _ = planned({}, after, max_files=1)
        self.assertEqual([f['path'] for f in by_files['files']], ['web/src/a.ts'])
        self.assertEqual([s['path'] for s in by_files['skipped']], ['web/src/b.ts', 'web/src/c.ts'])
        text = mutation_web.report_text(by_lines, {}, [], 10000, '.')
        self.assertIn('**Left out:** 1 file would have taken the run past 60 files or 10 lines: '
                      '`web/src/b.ts` (8 lines).', text)


class Summary(unittest.TestCase):
    def test_lists_what_no_test_detected_by_file_then_by_line(self):
        counts, found = mutation_web.undetected(REPORT)
        self.assertEqual(counts, {'Survived': 2, 'Killed': 2, 'Timeout': 1, 'NoCoverage': 1, 'CompileError': 1})
        with tempfile.TemporaryDirectory() as web:
            # b.ts has only a browser test; a.ts has a node test beside its browser test.
            write_tree(web, {'src/b.browser.test.tsx': '', 'src/a.browser.test.tsx': '', 'src/a.test.ts': ''})
            text = mutation_web.report_text(PLAN, counts, found, 100000, web)
        self.assertIn('| 2 | 1 | 2 | 1 | 1 | 50.0 % |', text)
        by_file = text[text.index('by file'):text.index('### By line')]
        self.assertIn('| `web/src/a.ts` | 1 | 1 |  |\n| `web/src/b.ts` | 1 | 0 | tested only in Chromium '
                      '(`b.browser.test.tsx`), which this run leaves out |', by_file)
        self.assertIn('write a test that fails on it, or say in a comment here that it is equivalent', text)
        self.assertIn('| `web/src/a.ts` | 3 | unreached | StringLiteral | <code>third</code> '
                      '| <code>&quot;&quot;</code> |', text)
        # A | in code would end the table cell.
        self.assertIn('| `web/src/b.ts` | 1 | survived | LogicalOperator | <code>a &#124; b</code> '
                      '| <code>a &amp; b</code> |', text)
        for limit in ('equivalent mutants', 'pins today', 'gates nothing', 'timeout', '59 of 94'):
            self.assertIn(limit, text)

    def test_a_long_list_shows_fifty_then_folds_then_points_to_the_artifact(self):
        counts, found = mutation_web.undetected(many(200))
        text = mutation_web.report_text(PLAN, counts, found, 100000, '.')
        shown, folded = text.split('<details>')
        self.assertEqual(shown.count('| survived |'), mutation_web.SHOWN)
        self.assertIn('<summary>150 more mutants</summary>', folded)
        self.assertEqual(folded.count('| survived |'), 150)
        self.assertNotIn('artifact `mutation-web`.', folded)
        short = mutation_web.report_text(PLAN, counts, found, 12000, '.')
        self.assertLessEqual(len(short), 12000)
        kept = short.count('| survived |')
        self.assertIn(f'{200 - kept} more are in the run\'s artifact `mutation-web`.', short)

    def test_writes_the_summary_and_an_issue_body_github_accepts(self):
        with tempfile.TemporaryDirectory() as tmp, contextlib.chdir(tmp):
            Path('report.json').write_text(json.dumps(REPORT))
            Path('plan.json').write_text(json.dumps(PLAN))
            env = {'GITHUB_STEP_SUMMARY': f'{tmp}/summary.md', 'GITHUB_OUTPUT': f'{tmp}/output'}
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('report.json', 'plan.json', tmp)
            self.assertIn('### 3 mutants no test detected, by file', Path('summary.md').read_text())
            self.assertEqual(Path('output').read_text(), 'undetected=3\n')
            info = json.loads(Path('issue.json').read_text())
            self.assertEqual({k: info[k] for k in ('count', 'survived', 'unreached', 'files', 'base', 'sha')},
                             {'count': 3, 'survived': 2, 'unreached': 1, 'files': 2, 'base': 'v0.14.0', 'sha': SHA})
            self.assertEqual(info['fingerprint'], mutation_web.fingerprint(mutation_web.undetected(REPORT)[1]))
            # 3000 long rows: the summary takes them, the issue body stays under GitHub's 65536.
            Path('big.json').write_text(json.dumps(many(3000, 'y' * 80)))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('big.json', 'plan.json', tmp)
            info = json.loads(Path('issue.json').read_text())
            body = mutation_web.decide(None, info, 'https://github.com/o/r/actions/runs/1')[0][2]
            self.assertLess(len(body), 65536)
            self.assertIn('more are in the run\'s artifact', body)
            # Nothing changed since the release: no report, and the summary says so.
            Path('empty.json').write_text(json.dumps({**PLAN, 'files': [], 'lines': 0}))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('missing.json', 'empty.json', tmp)
            info = json.loads(Path('issue.json').read_text())
            self.assertEqual((info['count'], info['files']), (0, 0))
            self.assertIn('No `web/src` lines changed since v0.14.0: nothing to mutate.', info['body'])
            # Lines to mutate but no report: Stryker did not finish.
            with mock.patch.dict(os.environ, env), self.assertRaises(FileNotFoundError):
                mutation_web.summary('missing.json', 'plan.json', tmp)


INFO = {'count': 3, 'survived': 2, 'unreached': 1, 'files': 2, 'base': 'v0.14.0',
        'fingerprint': 'f' * 64, 'sha': SHA, 'body': 'the list'}
RUN = 'https://github.com/o/r/actions/runs/1'


def issue_with(state, fingerprint='f' * 64):
    return {'number': 7, 'state': state, 'body': f'old\n<!-- mutation-web: {fingerprint} -->\n'}


class Issue(unittest.TestCase):
    def test_one_issue_is_opened_reopened_updated_or_closed(self):
        decide = mutation_web.decide
        nothing_changed = {**INFO, 'count': 0, 'survived': 0, 'unreached': 0, 'files': 0}
        all_detected = {**INFO, 'count': 0, 'survived': 0, 'unreached': 0}
        self.assertEqual(decide(None, nothing_changed, RUN), [])
        self.assertEqual(decide(issue_with('CLOSED'), all_detected, RUN), [])
        self.assertEqual(decide(issue_with('OPEN'), nothing_changed, RUN), [
            ('comment', 7, f'No `web/src` lines changed since v0.14.0 at {SHA[:12]}, so nothing was mutated: {RUN}'),
            ('close', 7, None)])
        self.assertEqual(decide(issue_with('OPEN'), all_detected, RUN), [
            ('comment', 7, f'A test detects every mutant of the changed lines at {SHA[:12]}: {RUN}'),
            ('close', 7, None)])
        [(action, number, body)] = decide(None, INFO, RUN)
        self.assertEqual((action, number), ('create', None))
        self.assertIn('the list', body)
        self.assertIn('Linear', body)
        self.assertIn(f'<!-- mutation-web: {"f" * 64} -->', body)
        # The same list: an open issue stays as it is, a closed one stays closed.
        self.assertEqual(decide(issue_with('OPEN'), INFO, RUN), [])
        self.assertEqual(decide(issue_with('CLOSED'), INFO, RUN), [])
        changed = decide(issue_with('OPEN', 'e' * 64), INFO, RUN)
        self.assertEqual([(a, n) for a, n, _ in changed], [('edit', 7), ('comment', 7)])
        self.assertIn('f' * 64, changed[0][2])
        self.assertIn('3 mutants no test detects', changed[1][2])
        self.assertIn('(2 survived, 1 unreached)', changed[1][2])
        back = decide(issue_with('CLOSED', 'e' * 64), INFO, RUN)
        self.assertEqual([(a, n) for a, n, _ in back], [('reopen', 7), ('edit', 7), ('comment', 7)])

    def run_issue(self, issues, dry_run):
        calls = []

        def fake_run(*args, stdin=None):
            calls.append((args, stdin))
            if args[:3] == ('gh', 'issue', 'list'):
                query = args[args.index('--search') + 1]
                hits = [i for i in issues if ('in:title' in query and mutation_web.TITLE in i['title'])
                        or ('in:body' in query and 'mutation-web' in i['body'])]
                return json.dumps(hits)
            return 'https://github.com/o/r/issues/9\n'
        env = {'GITHUB_REPOSITORY': 'o/r', 'GITHUB_SERVER_URL': 'https://github.com', 'GITHUB_RUN_ID': '1'}
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'issue.json').write_text(json.dumps(INFO))
            out = io.StringIO()
            with mock.patch.object(mutation_web, 'run', fake_run), mock.patch.dict(os.environ, env), \
                    contextlib.redirect_stdout(out):
                mutation_web.issue(f'{tmp}/issue.json', dry_run)
        return calls, out.getvalue()

    def test_a_scheduled_run_finds_its_issue_open_or_closed_and_writes_through_gh(self):
        similar = {'number': 2, 'title': f'{mutation_web.TITLE} (old)', 'body': '', 'state': 'OPEN'}
        calls, _ = self.run_issue([similar], dry_run=False)
        lists = [args for args, _ in calls if args[:3] == ('gh', 'issue', 'list')]
        self.assertEqual(len(lists), 2)
        for args in lists:
            self.assertEqual(args[args.index('--state') + 1], 'all')
        [(args, stdin)] = [c for c in calls if c[0][:3] != ('gh', 'issue', 'list')]
        self.assertEqual(args, ('gh', 'issue', 'create', '--repo', 'o/r', '--title', mutation_web.TITLE,
                                '--body-file', '-'))
        self.assertIn('the list', stdin)
        # The newest: closed after a release, renamed by hand, found by its marker.
        renamed = {'number': 5, 'title': 'Untested web lines', 'state': 'CLOSED',
                   'body': f'<!-- mutation-web: {"e" * 64} -->'}
        older = {'number': 3, 'title': mutation_web.TITLE, 'body': '', 'state': 'OPEN'}
        calls, _ = self.run_issue([similar, older, renamed], dry_run=False)
        self.assertEqual([args[:4] for args, _ in calls if args[:3] != ('gh', 'issue', 'list')],
                         [('gh', 'issue', 'reopen', '5'), ('gh', 'issue', 'edit', '5'), ('gh', 'issue', 'comment', '5')])

    def test_a_dry_run_only_reads(self):
        calls, out = self.run_issue([], dry_run=True)
        self.assertEqual({args[:3] for args, _ in calls}, {('gh', 'issue', 'list')})
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
        # Stryker's config and its pinned packages (a Dependabot update of them
        # too) and the Vitest config it runs.
        for path in ('.github/workflows/scripts/mutation_web.py', '.github/workflows/mutation-web.yml',
                     'web/scripts/mutation/**', 'web/vitest.config.ts', 'web/vitest.mutation.config.ts'):
            self.assertIn(path, paths)


if __name__ == '__main__':
    unittest.main()
