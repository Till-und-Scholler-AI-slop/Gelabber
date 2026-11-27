"""Caches that hand a build output from one job to another (GEL-394).

The libwebrtc jobs of desktop-native.yml look the package up, fetch or build
it on a miss, and save it under its key; the core jobs restore that key with
fail-on-cache-miss. So the save must run for every event the core jobs run
for, pull requests included, or a pull request that changes the key can
never pass.
"""
from pathlib import Path
import re
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[3]
# Inside a called workflow, github.event_name is the caller's event.
EVENTS = ('pull_request', 'merge_group', 'push', 'release', 'workflow_dispatch')


def excludes(condition, event):
    """Whether an `if:` names `event` as one it does not run for. Only
    github.event_name is read; any other github context fails the test,
    rather than passing a condition that may well skip pull requests."""
    text = str(condition or '')
    unknown = sorted(set(re.findall(r'\bgithub\.[\w.-]+', text)) - {'github.event_name'})
    if unknown:
        raise AssertionError(f'{text!r} reads {unknown}; this test cannot tell which events '
                             'that skips: teach excludes() or write it with github.event_name')
    return (re.search(rf"github\.event_name\s*!=\s*'{event}'", text) is not None
            or re.search(r"github\.event_name\s*==\s*'(?!" + event + r"')[\w-]+'", text) is not None)


class HandOver(unittest.TestCase):
    def test_every_required_cache_is_saved_for_the_events_that_need_it(self):
        workflow = yaml.safe_load((ROOT / '.github/workflows/desktop-native.yml').read_text())
        jobs = workflow['jobs']
        required = [(job_id, step) for job_id, job in jobs.items() for step in job.get('steps', [])
                    if step.get('uses', '').startswith('actions/cache/restore@')
                    and (step.get('with') or {}).get('fail-on-cache-miss') is True]
        self.assertTrue(required, 'no step restores a cache with fail-on-cache-miss')
        for job_id, step in required:
            with self.subTest(job=job_id):
                producer, output = re.fullmatch(r'\$\{\{ needs\.([\w-]+)\.outputs\.([\w-]+) \}\}',
                                                step['with']['key']).groups()
                self.assertIn(producer, jobs[job_id]['needs'] if isinstance(jobs[job_id]['needs'], list)
                              else [jobs[job_id]['needs']])
                key = jobs[producer]['outputs'][output]
                saves = [save for save in jobs[producer]['steps']
                         if re.match(r'actions/cache(?:/save)?@', save.get('uses', ''))
                         and (save.get('with') or {}).get('key') == key]
                self.assertEqual(len(saves), 1, f'{producer} saves {key} once')
                for event in (event for event in EVENTS if not excludes(jobs[job_id].get('if'), event)):
                    with self.subTest(job=job_id, event=event):
                        self.assertFalse(excludes(saves[0].get('if'), event),
                                         f'{job_id} restores {key} for {event}, but {producer} '
                                         'does not save it then')


if __name__ == '__main__':
    unittest.main()
