"""Web mutation testing picks the changed lines, reports what no test
detected, and keeps one issue current (.github/workflows/scripts/mutation_web.py,
mutation-web.yml, GEL-388).

The Stryker report below has the shape Stryker 10.1.1 writes
(`reports/mutation/mutation.json`, schema 1.0).
"""
import contextlib
import html
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

    def test_a_long_snippet_shows_where_original_and_mutant_differ(self):
        before = 'typeof value.first === "string" && typeof value.second === "string" && value.third'
        after = 'typeof value.first === "string" && typeof value.second === "string" || value.third'
        shown = mutation_web.snippets(before, after)
        self.assertNotEqual(shown[0], shown[1])
        self.assertIn('&amp;&amp; value.third', shown[0])
        self.assertIn('&#124;&#124; value.third', shown[1])
        for cell in shown:
            self.assertLessEqual(len(html.unescape(cell.removeprefix('<code>').removesuffix('</code>'))),
                                 mutation_web.SNIPPET)
        self.assertEqual(mutation_web.snippets('a', 'b'), ('<code>a</code>', '<code>b</code>'))

    def test_the_open_list_follows_the_file_table_and_shows_a_few_per_file(self):
        report = {'files': {'src/big.ts': many(20)['files']['src/a.ts'], 'src/small.ts': many(3)['files']['src/a.ts']}}
        counts, found = mutation_web.undetected(report)
        text = mutation_web.report_text(PLAN, counts, found, 100000, '.')
        shown, folded = text[text.index('### By line'):].split('<details>')
        self.assertEqual(shown.count('`web/src/big.ts`'), mutation_web.PER_FILE)
        self.assertEqual(shown.count('`web/src/small.ts`'), 3)
        self.assertLess(shown.index('`web/src/big.ts`'), shown.index('`web/src/small.ts`'))
        self.assertIn('<summary>15 more mutants</summary>', folded)

    def test_a_long_list_shows_fifty_then_folds_then_points_to_the_artifact(self):
        report = {'files': {f'src/f{n:02}.ts': many(10)['files']['src/a.ts'] for n in range(20)}}
        counts, found = mutation_web.undetected(report)
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
            self.assertEqual({k: info[k] for k in ('count', 'survived', 'unreached', 'files', 'skipped', 'base', 'sha')},
                             {'count': 3, 'survived': 2, 'unreached': 1, 'files': 2, 'skipped': 0,
                              'base': 'v0.14.0', 'sha': SHA})
            self.assertEqual(info['keys'], ['src/a.ts\x1fConditionalExpression\x1fsecon\x1ffalse',
                                            'src/a.ts\x1fStringLiteral\x1fthird\x1f""',
                                            'src/b.ts\x1fLogicalOperator\x1fa | b\x1fa & b'])
            # 3000 long rows: the summary takes them, the issue body stays under GitHub's 65536.
            Path('big.json').write_text(json.dumps(many(3000, 'y' * 80)))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('big.json', 'plan.json', tmp)
            info = json.loads(Path('issue.json').read_text())
            body = mutation_web.decide(None, info, None, RUN)[0][2]
            self.assertLess(len(body), 65536)
            self.assertIn('more are in the run\'s artifact', body)
            # Nothing mutable changed since the release: no report, and the summary says so.
            Path('empty.json').write_text(json.dumps({**PLAN, 'files': [], 'lines': 0}))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('missing.json', 'empty.json', tmp)
            info = json.loads(Path('issue.json').read_text())
            self.assertEqual((info['count'], info['files'], info['skipped']), (0, 0, 0))
            self.assertIn('No mutable `web/src` lines changed since v0.14.0.', info['body'])
            # Every changed file over the limits: nothing mutated, and not "nothing changed".
            over = {**PLAN, 'files': [], 'lines': 0,
                    'skipped': [{'path': 'web/src/big.ts', 'lines': 1600, 'reason': 'over the limit'}]}
            Path('over.json').write_text(json.dumps(over))
            with mock.patch.dict(os.environ, env):
                mutation_web.summary('missing.json', 'over.json', tmp)
            info = json.loads(Path('issue.json').read_text())
            self.assertEqual(info['skipped'], 1)
            self.assertIn('Every changed file was over the limits; nothing was mutated.', info['body'])
            self.assertNotIn('No mutable', info['body'])
            # Lines to mutate but no report: Stryker did not finish.
            with mock.patch.dict(os.environ, env), self.assertRaises(FileNotFoundError):
                mutation_web.summary('missing.json', 'plan.json', tmp)


KEYS = ['src/a.ts\x1fConditionalExpression\x1fx\x1ffalse', 'src/a.ts\x1fStringLiteral\x1f"a"\x1f""',
        'src/a.ts\x1fStringLiteral\x1f"a"\x1f""']
INFO = {'count': 3, 'survived': 2, 'unreached': 1, 'files': 2, 'skipped': 0, 'base': 'v0.14.0',
        'sha': SHA, 'keys': KEYS, 'body': 'the list'}
RUN = 'https://github.com/o/r/actions/runs/1'
OPEN, CLOSED = {'number': 7, 'state': 'OPEN'}, {'number': 7, 'state': 'CLOSED'}


class Decide(unittest.TestCase):
    def test_a_shifted_line_is_not_new(self):
        moved = {'files': {name: {**data, 'source': '\n' + data['source'],
                                  'mutants': [{**m, 'location': {k: {**v, 'line': v['line'] + 1}
                                                                 for k, v in m['location'].items()}}
                                              for m in data['mutants']]}
                           for name, data in REPORT['files'].items()}}
        keys = [mutation_web.mutant_key(item) for item in mutation_web.undetected(REPORT)[1]]
        moved_keys = [mutation_web.mutant_key(item) for item in mutation_web.undetected(moved)[1]]
        self.assertEqual(moved_keys, keys)
        self.assertEqual(mutation_web.decide(OPEN, {**INFO, 'keys': moved_keys}, keys, RUN), [])

    def test_a_subset_after_a_close_stays_closed(self):
        self.assertEqual(mutation_web.decide(CLOSED, {**INFO, 'count': 1, 'keys': KEYS[:1]}, KEYS, RUN), [])
        self.assertEqual(mutation_web.decide(CLOSED, INFO, KEYS, RUN), [])

    def test_one_new_mutant_reopens_and_says_one_new(self):
        # A second copy of a known key is new too: the keys are a multiset.
        for extra in ('src/b.ts\x1fBooleanLiteral\x1ftrue\x1ffalse', KEYS[0]):
            with self.subTest(extra=extra):
                info = {**INFO, 'count': 4, 'keys': KEYS + [extra]}
                actions = mutation_web.decide(CLOSED, info, KEYS, RUN)
                self.assertEqual([(a, n) for a, n, _ in actions], [('reopen', 7), ('edit', 7), ('comment', 7)])
                self.assertIn('1 new mutant no test detects', actions[2][2])
                self.assertIn('(4 in all: 2 survived, 1 unreached)', actions[2][2])
        actions = mutation_web.decide(OPEN, {**INFO, 'count': 4, 'keys': KEYS + [KEYS[0]]}, KEYS, RUN)
        self.assertEqual([(a, n) for a, n, _ in actions], [('edit', 7), ('comment', 7)])

    def test_fewer_mutants_update_the_list_without_a_comment(self):
        actions = mutation_web.decide(OPEN, {**INFO, 'count': 1, 'keys': KEYS[:1]}, KEYS, RUN)
        self.assertEqual([(a, n) for a, n, _ in actions], [('edit', 7)])
        self.assertEqual(mutation_web.decide(OPEN, INFO, list(reversed(KEYS)), RUN), [])

    def test_a_lost_state_counts_every_mutant_as_known(self):
        self.assertEqual(mutation_web.decide(CLOSED, INFO, None, RUN), [])
        self.assertEqual([(a, n) for a, n, _ in mutation_web.decide(OPEN, INFO, None, RUN)], [('edit', 7)])
        [(action, number, body)] = mutation_web.decide(None, INFO, None, RUN)
        self.assertEqual((action, number), ('create', None))
        self.assertIn('the list', body)
        self.assertIn('Linear', body)
        self.assertNotIn('<!--', body)

    def test_nothing_left_closes_but_a_cut_run_changes_nothing(self):
        nothing = {**INFO, 'count': 0, 'survived': 0, 'unreached': 0, 'keys': [], 'files': 0}
        self.assertEqual(mutation_web.decide(OPEN, nothing, KEYS, RUN), [
            ('comment', 7, f'No mutable `web/src` lines changed since v0.14.0 at {SHA[:12]}, so nothing was '
                           f'mutated: {RUN}'), ('close', 7, None)])
        self.assertEqual(mutation_web.decide(OPEN, {**nothing, 'files': 2}, KEYS, RUN), [
            ('comment', 7, f'A test detects every mutant of the changed lines at {SHA[:12]}: {RUN}'),
            ('close', 7, None)])
        self.assertEqual(mutation_web.decide(OPEN, {**nothing, 'skipped': 1}, KEYS, RUN), [])
        self.assertEqual(mutation_web.decide(CLOSED, {**nothing, 'files': 2}, KEYS, RUN), [])
        self.assertEqual(mutation_web.decide(None, nothing, None, RUN), [])


class FakeGitHub:
    """`gh` for issue(): issues and the earlier scheduled runs' states.
    `gh issue list` honours --limit and `author:` and returns the newest
    first, as `sort:created-desc` asks; an issue without `author` is the
    workflow's. A view is a state, or (state, title). A state that is a
    string is written as it is (malformed JSON)."""

    def __init__(self, issues=(), states=(), views=None):
        self.issues, self.states, self.views, self.calls = list(issues), dict(states), views, []

    def __call__(self, *args, stdin=None):
        self.calls.append((args, stdin))
        value = lambda flag: args[args.index(flag) + 1]
        if args[:3] == ('gh', 'issue', 'list'):
            newest = sorted(self.issues, key=lambda i: -i['number'])
            hits = [i for i in newest if mutation_web.TITLE.lower() in i['title'].lower()]
            if 'author:app/github-actions' in value('--search'):
                hits = [i for i in hits if i.get('author', 'app/github-actions') == 'app/github-actions']
            return json.dumps([{k: v for k, v in i.items() if k != 'author'} for i in hits[:int(value('--limit'))]])
        if args[:3] == ('gh', 'issue', 'view'):
            if self.views is None or int(args[3]) not in self.views:
                raise subprocess.CalledProcessError(1, args, stderr='Could not resolve to an issue')
            view = self.views[int(args[3])]
            state, title = view if isinstance(view, tuple) else (view, mutation_web.TITLE)
            return json.dumps({'number': int(args[3]), 'state': state, 'title': title})
        if args[:3] == ('gh', 'run', 'list'):
            # Only this workflow's scheduled runs that passed may hand on a state.
            assert (value('--workflow'), value('--event'), value('--status')) == \
                ('mutation-web.yml', 'schedule', 'success'), args
            return json.dumps([{'databaseId': run_id} for run_id in sorted(self.states, reverse=True)])
        if args[:3] == ('gh', 'run', 'download'):
            state = self.states.get(int(args[3]))
            if state is None:
                raise subprocess.CalledProcessError(1, args, stderr='no valid artifacts found to download')
            Path(value('--dir')).mkdir(parents=True, exist_ok=True)
            Path(value('--dir'), mutation_web.STATE_FILE).write_text(state if isinstance(state, str)
                                                                    else json.dumps(state))
            return ''
        if args[:3] == ('gh', 'issue', 'create'):
            return 'https://github.com/o/r/issues/9\n'
        return ''

    def writes(self):
        return [args[:4] for args, _ in self.calls if args[:2] == ('gh', 'issue') and args[2] not in ('list', 'view')]


class Issue(unittest.TestCase):
    def run_issue(self, fake, info=INFO, dry_run=False):
        env = {'GITHUB_REPOSITORY': 'o/r', 'GITHUB_SERVER_URL': 'https://github.com', 'GITHUB_RUN_ID': '100'}
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'issue.json').write_text(json.dumps(info))
            out = io.StringIO()
            with mock.patch.object(mutation_web, 'run', fake), mock.patch.dict(os.environ, env), \
                    contextlib.redirect_stdout(out):
                mutation_web.issue(f'{tmp}/issue.json', dry_run, f'{tmp}/state/mutation-state.json')
            return json.loads(Path(tmp, 'state/mutation-state.json').read_text()), out.getvalue()

    def test_the_first_run_opens_the_issue_and_keeps_its_number(self):
        fake = FakeGitHub()
        state, _ = self.run_issue(fake)
        self.assertEqual(fake.writes(), [('gh', 'issue', 'create', '--repo')])
        create = [c for c in fake.calls if c[0][:3] == ('gh', 'issue', 'create')][0]
        self.assertEqual(create[0], ('gh', 'issue', 'create', '--repo', 'o/r', '--title', mutation_web.TITLE,
                                     '--body-file', '-'))
        self.assertIn('the list', create[1])
        self.assertEqual(state, {'issue': 9, 'keys': KEYS})

    def test_the_last_state_names_the_issue_and_what_is_known(self):
        # Run 99 is this workflow's last scheduled run; 98's state is older.
        fake = FakeGitHub(states={99: {'issue': 7, 'keys': KEYS[:2]}, 98: {'issue': 3, 'keys': []}},
                          views={7: 'CLOSED'})
        state, out = self.run_issue(fake)
        self.assertEqual(fake.writes(), [('gh', 'issue', 'reopen', '7'), ('gh', 'issue', 'edit', '7'),
                                         ('gh', 'issue', 'comment', '7')])
        comment = [stdin for args, stdin in fake.calls if args[:3] == ('gh', 'issue', 'comment')][0]
        self.assertIn('1 new mutant', comment)
        self.assertEqual(state, {'issue': 7, 'keys': KEYS})
        self.assertIn('last state: 2 mutant keys', out)
        # The same keys again: nothing.
        fake = FakeGitHub(states={99: {'issue': 7, 'keys': KEYS}}, views={7: 'OPEN'})
        self.run_issue(fake)
        self.assertEqual(fake.writes(), [])
        # This run's own state is not the last one.
        fake = FakeGitHub(states={100: {'issue': 7, 'keys': []}, 99: {'issue': 7, 'keys': KEYS}}, views={7: 'OPEN'})
        self.run_issue(fake)
        self.assertEqual(fake.writes(), [])

    def test_no_earlier_artifact_finds_the_newest_issue_by_title_and_pings_nobody(self):
        similar = {'number': 12, 'title': f'Not quite: {mutation_web.TITLE}', 'state': 'OPEN'}
        older = {'number': 4, 'title': mutation_web.TITLE, 'state': 'OPEN'}
        newest = {'number': 8, 'title': mutation_web.TITLE, 'state': 'CLOSED'}
        # Expired: the run is listed, its artifact is gone.
        fake = FakeGitHub(issues=[similar, older, newest], states={99: None})
        state, out = self.run_issue(fake)
        self.assertEqual(fake.writes(), [])
        self.assertIn('Issue "Mutation testing: web mutants survive": #8 (closed); last state: none', out)
        self.assertEqual(state, {'issue': 8, 'keys': KEYS})
        for args, _ in fake.calls:
            if args[:3] == ('gh', 'issue', 'list'):
                self.assertEqual(args[args.index('--state') + 1], 'all')
                self.assertIn('sort:created-desc', args[args.index('--search') + 1])
        fake = FakeGitHub(issues=[similar, older], states={})
        self.run_issue(fake)
        self.assertEqual(fake.writes(), [('gh', 'issue', 'edit', '4')])

    def test_a_run_over_the_limits_keeps_the_last_state(self):
        over = {**INFO, 'count': 0, 'survived': 0, 'unreached': 0, 'keys': [], 'files': 0, 'skipped': 1}
        fake = FakeGitHub(states={99: {'issue': 7, 'keys': KEYS}}, views={7: 'OPEN'})
        state, _ = self.run_issue(fake, over)
        self.assertEqual(fake.writes(), [])
        self.assertEqual(state, {'issue': 7, 'keys': KEYS})

    def test_a_lost_state_stays_lost_across_a_run_over_the_limits(self):
        mine = {'number': 8, 'title': mutation_web.TITLE, 'state': 'OPEN'}
        over = {**INFO, 'count': 0, 'survived': 0, 'unreached': 0, 'keys': [], 'files': 0, 'skipped': 1}
        fake = FakeGitHub(issues=[mine])
        state, _ = self.run_issue(fake, over)
        self.assertEqual((fake.writes(), state), ([], {'issue': 8, 'keys': None}))
        # The next normal run: the state is still lost, so it updates the list and pings nobody.
        fake = FakeGitHub(issues=[mine], states={99: state}, views={8: 'OPEN'})
        state, out = self.run_issue(fake)
        self.assertEqual(fake.writes(), [('gh', 'issue', 'edit', '8')])
        self.assertIn('last state: none', out)
        self.assertEqual(state, {'issue': 8, 'keys': KEYS})

    def test_a_bad_newest_state_is_a_lost_state_not_an_older_one(self):
        mine = {'number': 7, 'title': mutation_web.TITLE, 'state': 'CLOSED'}
        older = {'issue': 7, 'keys': []}
        for newest in ({'issue': '7', 'keys': KEYS[:2]}, {'issue': 7, 'keys': 'x'}, {'issue': 7, 'keys': [1]},
                       {'issue': True, 'keys': KEYS[:2]}, '{"issue": 7, "keys": [', None):
            with self.subTest(newest=newest):
                fake = FakeGitHub(issues=[mine], states={99: newest, 98: older}, views={7: 'CLOSED'})
                state, out = self.run_issue(fake)
                self.assertEqual(fake.writes(), [])
                self.assertIn('#7 (closed); last state: none', out)
                self.assertEqual(state, {'issue': 7, 'keys': KEYS})
                self.assertNotIn(('gh', 'run', 'download', '98'), [args[:4] for args, _ in fake.calls])

    def test_only_this_workflows_issue_is_taken(self):
        # A number that now names another issue (transferred, or wrong): the title search decides.
        fake = FakeGitHub(issues=[{'number': 5, 'title': mutation_web.TITLE, 'state': 'OPEN'}],
                          states={99: {'issue': 7, 'keys': KEYS}}, views={7: ('OPEN', 'Something else')})
        state, out = self.run_issue(fake)
        self.assertEqual(fake.writes(), [])
        self.assertIn('#5 (open); last state: 3 mutant keys', out)
        self.assertEqual(state['issue'], 5)
        self.assertIn('author:app/github-actions', [args[args.index('--search') + 1] for args, _ in fake.calls
                                                    if args[:3] == ('gh', 'issue', 'list')][0])
        # A stranger's newer issue with the same title is not adopted.
        stranger = {'number': 12, 'title': mutation_web.TITLE, 'state': 'OPEN', 'author': 'someone'}
        fake = FakeGitHub(issues=[stranger, {'number': 4, 'title': mutation_web.TITLE, 'state': 'CLOSED'}])
        _, out = self.run_issue(fake)
        self.assertIn('#4 (closed)', out)
        self.assertEqual(fake.writes(), [])

    def test_a_dry_run_only_reads(self):
        fake = FakeGitHub(states={99: {'issue': 7, 'keys': KEYS[:2]}}, views={7: 'CLOSED'})
        _, out = self.run_issue(fake, dry_run=True)
        self.assertEqual(fake.writes(), [])
        self.assertIn('Would reopen #7', out)
        self.assertIn('Would comment #7:\n1 new mutant', out)


class Workflow(unittest.TestCase):
    def test_only_the_scheduled_runs_report_job_writes_the_issue(self):
        workflow = load('mutation-web.yml')
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        jobs = workflow['jobs']
        self.assertEqual({job_id for job_id, job in jobs.items() if writes(job.get('permissions'))}, {'report'})
        self.assertEqual(writes(jobs['report']['permissions']), {'issues'})
        self.assertEqual(jobs['report']['if'], "github.event_name == 'schedule'")
        # The next scheduled run reads this run's state from the artifact the report job keeps.
        self.assertEqual(jobs['report']['permissions'], {'actions': 'read', 'contents': 'read', 'issues': 'write'})
        [keep] = [step for step in jobs['report']['steps'] if step.get('uses', '').startswith('actions/upload-artifact@')]
        self.assertEqual(keep['with']['name'], mutation_web.STATE_ARTIFACT)
        self.assertEqual(keep['with']['retention-days'], mutation_web.STATE_DAYS)
        [write] = [step for step in jobs['report']['steps'] if 'mutation_web.py issue' in step.get('run', '')]
        self.assertTrue(write['run'].endswith(' ' + keep['with']['path']))
        self.assertEqual(Path(keep['with']['path']).name, mutation_web.STATE_FILE)
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
