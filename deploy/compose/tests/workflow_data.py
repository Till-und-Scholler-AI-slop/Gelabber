"""The workflows under .github/workflows as parsed data, for the tests.

PyYAML reads YAML 1.1, where a bare `on` is the boolean true; `load` puts
the triggers back under 'on'.
"""
import functools
from pathlib import Path
import re

import yaml

ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / '.github/workflows'


@functools.cache
def load(name):
    """A workflow file's data; callers must not change it."""
    data = yaml.safe_load((WORKFLOWS / name).read_text())
    if True in data:
        data['on'] = data.pop(True)
    return data


def names():
    return sorted(path.name for path in WORKFLOWS.glob('*.yml'))


def triggers(workflow):
    """The events that start a workflow, as a set of names."""
    on = workflow.get('on') or {}
    if isinstance(on, str):
        return {on}
    return set(on)


def writes(permissions):
    """The scopes a `permissions:` value grants write to."""
    if permissions in ('write-all',):
        return {'all'}
    if not isinstance(permissions, dict):
        return set()
    return {scope for scope, access in permissions.items() if access == 'write'}


def local_call(job):
    """The workflow file a job calls, or None."""
    uses = job.get('uses', '')
    return uses.removeprefix('./.github/workflows/') if uses.startswith('./.github/workflows/') else None


def steps(workflow):
    """(job id, step) for every step of every job."""
    return [(job_id, step) for job_id, job in workflow.get('jobs', {}).items()
            for step in job.get('steps', [])]


def scripts(workflow):
    """(job id, step name, script) for every `run:` step."""
    return [(job_id, step.get('name', ''), str(step['run'])) for job_id, step in steps(workflow) if 'run' in step]


def _split(text, operator):
    """`text` split at `operator` outside parentheses and quotes."""
    parts, depth, quote, start, index = [], 0, False, 0, 0
    while index < len(text):
        char = text[index]
        if char == "'":
            quote = not quote
        elif not quote and char == '(':
            depth += 1
        elif not quote and char == ')':
            depth -= 1
        elif not quote and depth == 0 and text.startswith(operator, index):
            parts.append(text[start:index])
            start = index + len(operator)
            index = start
            continue
        index += 1
    parts.append(text[start:])
    return [part.strip() for part in parts]


def _unwrap(term):
    """The term without parentheses around all of it."""
    while term.startswith('(') and term.endswith(')'):
        depth = 0
        for index, char in enumerate(term):
            depth += {'(': 1, ')': -1}.get(char, 0)
            if depth == 0 and index < len(term) - 1:
                return term
        term = term[1:-1].strip()
    return term


def _excludes(term, event):
    """Whether one condition is false for every run of `event`."""
    term = _unwrap(term)
    alternatives = _split(term, '||')
    if len(alternatives) > 1:
        return all(_excludes(alternative, event) for alternative in alternatives)
    conjuncts = _split(term, '&&')
    if len(conjuncts) > 1:
        return any(_excludes(conjunct, event) for conjunct in conjuncts)
    match = re.fullmatch(r"github\.event_name\s*(==|!=)\s*'([\w-]+)'", term)
    if match:
        return (match.group(1) == '!=') == (match.group(2) == event)
    # A pull request's or merge queue's ref is refs/pull/... or the queue's
    # own branch, never a named branch or a tag.
    if event in ('pull_request', 'merge_group'):
        if re.fullmatch(r"github\.ref\s*==\s*'refs/(?:heads/(?!gh-readonly-queue/)[\w./-]+|tags/[\w./-]+)'", term):
            return True
        if re.fullmatch(r"startsWith\(github\.ref,\s*'refs/tags/'\)", term):
            return True
    return False


def skips(job, event):
    """Whether a job's `if:` keeps it from running for `event`."""
    condition = job.get('if')
    if condition is None or isinstance(condition, bool):
        return condition is False
    text = ' '.join(str(condition).split())
    if text.startswith('${{') and text.endswith('}}'):
        text = text[3:-2].strip()
    return _excludes(text, event)
