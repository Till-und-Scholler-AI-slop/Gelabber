"""Which pull request run a push to main reuses (.github/workflows/scripts/tested_tree.py)."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/tested_tree.py'
REPO = 'Till-und-Scholler-AI-slop/Gelabber'
SHA = 'a' * 40
HEAD = 'b' * 40
TREE = 'c' * 40


def load():
    spec = importlib.util.spec_from_file_location('tested_tree', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


tested_tree = load()


def pull(**changes):
    found = {'number': 740, 'merged_at': '2026-11-26T20:00:00Z', 'merge_commit_sha': SHA,
             'base': {'ref': 'main'}, 'head': {'sha': HEAD, 'repo': {'full_name': REPO}}}
    found.update(changes)
    return found


def run(number, started='2026-11-26T19:00:00Z', **changes):
    found = {'id': number, 'path': '.github/workflows/ci.yml', 'event': 'pull_request',
             'status': 'completed', 'conclusion': 'success', 'head_sha': HEAD,
             'run_started_at': started,
             'head_repository': {'full_name': REPO}, 'repository': {'full_name': REPO}}
    found.update(changes)
    return found


ALL = {'backend': 'true', 'web': 'true', 'desktop': 'true', 'deploy': 'true', 'minio': 'true',
       'images': '[]'}


class RecordTest(unittest.TestCase):
    def test_records_the_areas_the_run_checked(self):
        record = tested_tree.record({**ALL, 'web': 'false'}, SHA, TREE)
        self.assertEqual(record['tree'], TREE)
        self.assertEqual(record['areas'], ['backend', 'deploy', 'desktop', 'minio'])


class MergedPullTest(unittest.TestCase):
    def test_the_merge_into_main_from_this_repository(self):
        self.assertEqual(tested_tree.merged_pull([pull()], SHA, REPO)['number'], 740)

    def test_nothing_else_counts(self):
        for other in (pull(merged_at=None), pull(merge_commit_sha='d' * 40),
                      pull(base={'ref': 'v0.14'}),
                      pull(head={'sha': HEAD, 'repo': {'full_name': 'someone/Gelabber'}}),
                      pull(head={'sha': HEAD, 'repo': None})):
            with self.subTest(other=other):
                self.assertIsNone(tested_tree.merged_pull([other], SHA, REPO))


class CandidateRunsTest(unittest.TestCase):
    def test_successful_ci_runs_of_the_last_head_newest_first(self):
        runs = [run(1, '2026-11-26T18:00:00Z'), run(2, '2026-11-26T19:00:00Z')]
        self.assertEqual([found['id'] for found in tested_tree.candidate_runs(runs, pull(), REPO)], [2, 1])

    def test_other_runs_do_not_count(self):
        for other in (run(1, conclusion='failure'), run(1, status='in_progress'),
                      run(1, event='push'), run(1, path='.github/workflows/e2e-extended.yml'),
                      run(1, head_sha='e' * 40), run(1, head_repository={'full_name': 'someone/Gelabber'})):
            with self.subTest(other=other):
                self.assertEqual(tested_tree.candidate_runs([other], pull(), REPO), [])


class StandsForTest(unittest.TestCase):
    def test_same_tree_with_every_area(self):
        self.assertTrue(tested_tree.stands_for(tested_tree.record(ALL, SHA, TREE), TREE))

    def test_another_tree_or_fewer_areas_are_no_reuse(self):
        self.assertFalse(tested_tree.stands_for(tested_tree.record(ALL, SHA, TREE), 'f' * 40))
        self.assertFalse(tested_tree.stands_for(tested_tree.record({**ALL, 'desktop': 'false'}, SHA, TREE), TREE))
        self.assertFalse(tested_tree.stands_for({'tree': TREE, 'areas': list(tested_tree.AREAS)}, TREE))


class CandidateTest(unittest.TestCase):
    """desktop-release.yml signs a release PR's packages only if it made them."""

    def test_a_candidate_run_stands_for_the_tag(self):
        tested = tested_tree.record(ALL, SHA, TREE, candidate=True)
        self.assertTrue(tested_tree.stands_for(tested, TREE, candidate=True))
        self.assertTrue(tested_tree.stands_for(tested, TREE))

    def test_a_run_without_packages_does_not(self):
        self.assertFalse(tested_tree.stands_for(tested_tree.record(ALL, SHA, TREE), TREE, candidate=True))
        self.assertFalse(tested_tree.stands_for(tested_tree.record(ALL, SHA, 'f' * 40, candidate=True),
                                                TREE, candidate=True))


class QueueTest(unittest.TestCase):
    """A push to a version branch reuses the merge queue's run (GEL-362)."""

    BEFORE = 'd' * 40

    def test_the_same_tree_on_the_same_base(self):
        tested = tested_tree.record({'web': 'true'}, SHA, TREE, base=self.BEFORE)
        self.assertTrue(tested_tree.stands_for_push(tested, TREE, self.BEFORE))
        self.assertFalse(tested_tree.stands_for_push(tested, TREE, 'e' * 40))
        self.assertFalse(tested_tree.stands_for_push(tested, 'f' * 40, self.BEFORE))

    def test_cache_inputs_need_a_push_run(self):
        for path in ('backend/Cargo.lock', 'desktop/Cargo.toml', 'web/package-lock.json',
                     'rust-toolchain.toml', 'backend/media/Dockerfile', 'desktop/native/libwebrtc.env',
                     '.github/workflows/ci.yml'):
            with self.subTest(path=path):
                self.assertTrue(tested_tree.touches_cache_inputs(['web/src/main.tsx', path]))
        self.assertFalse(tested_tree.touches_cache_inputs(['web/src/main.tsx', 'backend/api/src/lib.rs', '']))

    def test_queue_runs_of_the_pushed_commit_come_first(self):
        runs = [run(1, '2026-11-26T19:00:00Z', event='merge_group', head_sha='9' * 40),
                run(2, '2026-11-26T18:00:00Z', event='merge_group', head_sha=SHA),
                run(3, '2026-11-26T20:00:00Z', event='merge_group', head_sha='8' * 40),
                run(4, '2026-11-26T21:00:00Z', event='pull_request', head_sha=SHA)]
        self.assertEqual([found['id'] for found in tested_tree.queue_runs(runs, REPO, SHA)], [2, 3, 1])


class LastGoodTest(unittest.TestCase):
    """A push's changes start at the last commit whose push run passed (GEL-363)."""

    def test_the_newest_passed_push_run_on_an_ancestor(self):
        runs = [run(1, '2026-11-26T18:00:00Z', event='push', head_sha='1' * 40),
                run(2, '2026-11-26T19:00:00Z', event='push', head_sha='2' * 40),
                run(3, '2026-11-26T20:00:00Z', event='push', head_sha='3' * 40, conclusion='cancelled'),
                run(4, '2026-11-26T21:00:00Z', event='push', head_sha='4' * 40)]
        ancestors = {'1' * 40, '2' * 40, '3' * 40}
        self.assertEqual(tested_tree.last_good(runs, REPO, SHA, ancestors.__contains__), '2' * 40)

    def test_the_pushed_commit_itself_and_other_events_do_not_count(self):
        runs = [run(1, event='push', head_sha=SHA), run(2, event='pull_request', head_sha='1' * 40)]
        self.assertIsNone(tested_tree.last_good(runs, REPO, SHA, lambda commit: True))


class WorkflowTest(unittest.TestCase):
    """ci.yml records the tree on pull requests and reuses it on main only."""

    def setUp(self):
        self.ci = (ROOT / '.github/workflows/ci.yml').read_text()

    def test_reuse_only_on_pushes(self):
        reuse = self.ci.split('      - name: Run that passed this tree\n', 1)[1].split('\n      - name:', 1)[0]
        self.assertIn("if: github.event_name == 'push'\n", reuse)
        self.assertIn('if [ "$GITHUB_REF" = refs/heads/main ]; then\n'
                      '            python3 .github/workflows/scripts/tested_tree.py find >>', reuse)

    def test_merge_queue_runs_are_checked_and_recorded(self):
        self.assertIn('\n  merge_group:\n', self.ci)
        self.assertIn("github.event_name == 'pull_request' || github.event_name == 'merge_group'", self.ci)
        self.assertIn('tested_tree.py find-queue', self.ci)

    def test_pull_requests_record_their_tree(self):
        self.assertIn('tested_tree.py record', self.ci)
        self.assertIn('name: tested-tree', self.ci)


if __name__ == '__main__':
    unittest.main()
