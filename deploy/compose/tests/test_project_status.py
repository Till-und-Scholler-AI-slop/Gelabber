"""Which project Status moves an event causes (.github/workflows/scripts/project_status.py)."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('project_status', ROOT / '.github/workflows/scripts/project_status.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
WORKFLOW = ROOT / '.github/workflows/project-status.yml'


def pr(action, body='Closes #12', draft=False, merged=False, base='v0.11'):
    return {'action': action, 'pull_request': {
        'body': body, 'draft': draft, 'merged': merged, 'base': {'ref': base}}}


def decide(event_name, event):
    return m.decide(event_name, event, 'main')


class ClosingRefs(unittest.TestCase):
    def test_keywords_case_and_colon(self):
        body = 'Closes #1, fixes: #2\nResolved #3 and fix #1 again; see #9'
        self.assertEqual(m.closing_refs(body), [1, 2, 3])

    def test_mentions_and_other_words_are_not_refs(self):
        self.assertEqual(m.closing_refs('Part of #5, enclosed #6, closest #7'), [])
        self.assertEqual(m.closing_refs(None), [])


class Decide(unittest.TestCase):
    def test_assigning_starts_work(self):
        steps = decide('issues', {'action': 'assigned', 'issue': {'number': 4, 'assignees': [{}]}})
        self.assertEqual(steps, [m.move(4, 'In progress', m.NOT_STARTED)])

    def test_removing_the_last_assignee_puts_it_back(self):
        last = decide('issues', {'action': 'unassigned', 'issue': {'number': 4, 'assignees': []}})
        self.assertEqual(last, [m.move(4, m.RESTING, ('In progress',))])
        other_left = decide('issues', {'action': 'unassigned', 'issue': {'number': 4, 'assignees': [{}]}})
        self.assertEqual(other_left, [])

    def test_ready_pull_request_moves_to_in_review(self):
        for action in ('opened', 'reopened', 'ready_for_review', 'edited'):
            steps = decide('pull_request', pr(action))
            self.assertEqual(steps, [m.move(12, 'In review', m.NOT_STARTED + ('In progress',))])

    def test_draft_pull_request_only_starts_work(self):
        steps = decide('pull_request', pr('opened', draft=True))
        self.assertEqual(steps, [m.move(12, 'In progress', m.NOT_STARTED)])
        back = decide('pull_request', pr('converted_to_draft', draft=True))
        self.assertEqual(back, [m.move(12, 'In progress', ('In review',))])

    def test_merge_into_a_version_branch_closes_the_issue(self):
        steps = decide('pull_request', pr('closed', merged=True, base='v0.11'))
        self.assertEqual(steps, [m.close(12), m.move(12, 'Done')])

    def test_merge_into_the_default_branch_leaves_closing_to_github(self):
        steps = decide('pull_request', pr('closed', merged=True, base='main'))
        self.assertEqual(steps, [m.move(12, 'Done')])

    def test_closing_without_merge_takes_it_out_of_review(self):
        steps = decide('pull_request', pr('closed'))
        self.assertEqual(steps, [m.move(12, m.RESTING, ('In review', 'In progress'))])

    def test_pull_request_without_closing_keyword_moves_nothing(self):
        self.assertEqual(decide('pull_request', pr('opened', body='Part of #12')), [])

    def test_manual_dispatch_sets_any_status(self):
        steps = decide('workflow_dispatch', {'inputs': {'issue': '7', 'status': 'In review'}})
        self.assertEqual(steps, [m.move(7, 'In review')])
        with self.assertRaises(ValueError):
            decide('workflow_dispatch', {'inputs': {'issue': '7', 'status': 'Review'}})

    def test_resting_status(self):
        self.assertEqual(m.resting_status({'assignees': [{}], 'milestone': None}), 'In progress')
        self.assertEqual(m.resting_status({'assignees': [], 'milestone': {'number': 4}}), 'Planned')
        self.assertEqual(m.resting_status({'assignees': [], 'milestone': None}), 'Backlog')


class Workflow(unittest.TestCase):
    def test_dispatch_offers_exactly_the_known_statuses(self):
        text = WORKFLOW.read_text()
        self.assertIn('options: [' + ', '.join(m.STATUSES) + ']', text)

    def test_listens_to_every_pull_request_action_decide_handles(self):
        text = WORKFLOW.read_text()
        for action in ('opened', 'reopened', 'ready_for_review', 'converted_to_draft', 'edited', 'closed'):
            self.assertIn(action, text)


if __name__ == '__main__':
    unittest.main()
