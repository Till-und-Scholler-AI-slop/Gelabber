"""Flaky tests in nextest's JUnit reports (GEL-389).

CI runs a failed test once more (nextest profile `ci`); one that then passes
is flaky and passes the run, so a flake costs no re-run. It must not pass
silently, though:

`summary JUNIT...` writes every flaky test to the job summary
($GITHUB_STEP_SUMMARY) and as a warning on the run. A report it is named
but cannot find is an error: retries would otherwise pass flakes unseen.

`issues JUNIT|FOLDER...` (push runs, with an `issues: write` token) adds
each flake to its issue: the one .github/flaky-tests.toml names for the
test, else the open issue "Flaky test: <test>", which it opens the first
time, at most OPEN_LIMIT per run. A missing report is skipped (a job that
did not run uploads none), and a failed GitHub request is a warning only:
recording must not turn a push run red.

`check` validates .github/flaky-tests.toml: every entry names its issue, and
the list holds at most CAP entries, so it cannot become where flakes live.
"""
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ElementTree

ROOT = Path(__file__).resolve().parents[3]
QUARANTINE = ROOT / '.github/flaky-tests.toml'
CAP = 5
TITLE = 'Flaky test: '
# GitHub refuses longer titles.
TITLE_LIMIT = 256
# What a comment or warning shows of the first failure.
MESSAGE_LIMIT = 2000
WARNING_LINES = 4
# Issues one run opens at most: a broken service can make many tests flaky.
OPEN_LIMIT = 3
LINEAR = 'Add this issue to the Linear project Gelabber, label Bug.'


@dataclass(frozen=True)
class Flake:
    """A test that failed and then passed: `test` is nextest's
    `<binary-id> <name>`, `message` the first failure's text."""
    test: str
    attempts: int
    message: str


def flakes(junit):
    """The flaky tests in one JUnit report's text, in report order."""
    found = []
    for case in ElementTree.fromstring(junit).iter('testcase'):
        # Failures before the passing try; a test that failed in the end has
        # <failure> and <rerunFailure> instead.
        failures = case.findall('flakyFailure')
        if not failures:
            continue
        first = failures[0]
        message = (first.text or first.get('message') or '').strip()
        found.append(Flake(f"{case.get('classname')} {case.get('name')}", len(failures) + 1,
                           message[:MESSAGE_LIMIT]))
    return found


def read_reports(paths):
    """The flakes of the reports at `paths`, each a report or a folder of
    them (the downloaded artifacts), and the paths that do not exist."""
    found, missing = [], []
    for path in map(Path, paths):
        if path.is_dir():
            reports = sorted(path.rglob('*.xml'))
        elif path.is_file():
            reports = [path]
        else:
            missing.append(str(path))
            continue
        for report in reports:
            found.extend(flakes(report.read_text(encoding='utf-8')))
    return found, missing


def quarantine(text):
    """The known flaky tests, test -> issue number, from flaky-tests.toml's
    text. Raises ValueError when an entry lacks its issue or the list is
    longer than CAP."""
    entries = tomllib.loads(text).get('test', [])
    known = {}
    for entry in entries:
        test, issue = entry.get('id'), entry.get('issue')
        if not isinstance(test, str) or not test.strip():
            raise ValueError(f'entry without a test id: {entry}')
        if isinstance(issue, bool) or not isinstance(issue, int) or issue <= 0:
            raise ValueError(f'{test}: `issue` must be the number of its GitHub issue')
        if test in known:
            raise ValueError(f'{test} is listed twice')
        known[test] = issue
    if len(known) > CAP:
        raise ValueError(f'{len(known)} known flaky tests, at most {CAP}: fix one before you add one')
    return known


def title(test):
    return (TITLE + test)[:TITLE_LIMIT]


def fence(text):
    """`text` as a Markdown code block that its own backticks cannot end."""
    longest = max((len(run) for run in re.findall('`+', text)), default=0)
    ticks = '`' * max(3, longest + 1)
    return f'{ticks}\n{text}\n{ticks}'


def summary(found, known, job):
    """The job summary's Markdown: every flaky test, and where it is tracked."""
    if not found:
        return ''
    lines = [f'### Flaky tests in {job}', '',
             f'{len(found)} test(s) failed and then passed on retry; the run counts them as passed.', '',
             '| Test | Attempts | Tracked in |', '|---|---|---|']
    for flake in found:
        issue = known.get(flake.test)
        where = f'#{issue}' if issue else 'not on .github/flaky-tests.toml: a push run opens an issue'
        lines.append(f'| `{flake.test}` | {flake.attempts} | {where} |')
    lines.append('')
    for flake in found:
        lines += [f'<details><summary><code>{flake.test}</code>: first failure</summary>', '',
                  fence(flake.message), '', '</details>', '']
    return '\n'.join(lines)


def escape_data(text):
    """A workflow command's message, which must stay on one line."""
    return text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')


def warnings(found):
    """One `::warning` line per flaky test, shown on the run's page; the
    failure's first lines (where it panicked, then what) follow in the text."""
    lines = []
    for flake in found:
        head = '\n'.join(flake.message.splitlines()[:WARNING_LINES])
        lines.append('::warning title=Flaky test::' +
                     escape_data(f'{flake.test} failed, then passed on retry:\n{head}'))
    return lines


def plan(found, known, open_issues, run):
    """What `issues` does, one action per flaky test: ('comment', number,
    body), ('open', title, body), or ('skip', title, body) for an issue past
    OPEN_LIMIT or when the open issues are unknown (`open_issues` None).
    `open_issues` maps open issues' titles to numbers; `run` describes the
    run (url, branch, sha)."""
    actions = []
    opened = 0
    for flake in dict((flake.test, flake) for flake in found).values():
        body = (f'`{flake.test}` failed and then passed on retry ({flake.attempts} attempts) '
                f'in [this run]({run["url"]}) on `{run["branch"]}` at {run["sha"]}.\n\n'
                f'First failure:\n\n{fence(flake.message)}\n')
        number = known.get(flake.test) or (open_issues or {}).get(title(flake.test))
        if number:
            actions.append(('comment', number, body))
            continue
        body += ('\nCI retries a failed test once and passes it when the retry passes (GEL-389). '
                 'Further flakes of this test are added here. Fix the test, or, while it is being '
                 'fixed, list it in `.github/flaky-tests.toml` with the issue that tracks it.\n\n'
                 f'{LINEAR}\n')
        if open_issues is None or opened >= OPEN_LIMIT:
            actions.append(('skip', title(flake.test), body))
        else:
            opened += 1
            actions.append(('open', title(flake.test), body))
    return actions


def gh_api(*args):
    return subprocess.run(('gh', 'api', *args), check=True, capture_output=True, text=True).stdout


def open_flaky_issues(repo):
    """Open issues titled "Flaky test: …", title -> number."""
    found = {}
    pages = gh_api('--paginate', '--slurp', f'repos/{repo}/issues?state=open&per_page=100')
    for page in json.loads(pages):
        for issue in page:
            if 'pull_request' not in issue and issue['title'].startswith(TITLE):
                found[issue['title']] = issue['number']
    return found


def file_issues(actions, repo):
    """Carries out the actions; returns the job summary's lines."""
    lines = []
    for action, target, body in actions:
        try:
            if action == 'comment':
                gh_api('--method', 'POST', f'repos/{repo}/issues/{target}/comments', '-f', f'body={body}')
                lines.append(f'- Added to #{target}')
            elif action == 'open':
                created = json.loads(gh_api('--method', 'POST', f'repos/{repo}/issues',
                                            '-f', f'title={target}', '-f', f'body={body}'))
                lines.append(f'- Opened #{created["number"]}: {target}')
            else:
                lines.append(f'- Not opened (at most {OPEN_LIMIT} new issues per run): {target}')
        except (subprocess.CalledProcessError, ValueError, KeyError) as error:
            print(f'::warning title=Flaky test not recorded::{escape_data(f"{target}: {error}")}')
            lines.append(f'- Not recorded (GitHub request failed): {target}')
    return lines


def append_summary(text):
    if text and os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a', encoding='utf-8') as out:
            out.write(text + '\n')


def main(argv):
    command, paths = (argv[0], argv[1:]) if argv else ('', [])
    known = quarantine(QUARANTINE.read_text(encoding='utf-8'))
    if command == 'check':
        print(f'{len(known)} known flaky test(s), at most {CAP}.')
        return 0
    found, missing = read_reports(paths)
    if command == 'summary':
        if missing or not paths:
            for path in missing or ['(none named)']:
                print(f'::error title=Flaky tests::no JUnit report at {escape_data(path)}')
            return 1
        print('\n'.join(warnings(found)) or 'No flaky tests.')
        append_summary(summary(found, known, os.environ.get('JOB_LABEL', 'this job')))
        return 0
    if command == 'issues':
        repo = os.environ['GITHUB_REPOSITORY']
        run = {'url': f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
               'branch': os.environ['GITHUB_REF_NAME'], 'sha': os.environ['GITHUB_SHA']}
        open_issues = {}
        if found:
            try:
                open_issues = open_flaky_issues(repo)
            except (subprocess.CalledProcessError, ValueError, KeyError) as error:
                print(f'::warning title=Open issues unknown::{escape_data(str(error))}: no issue is opened')
                open_issues = None
        lines = file_issues(plan(found, known, open_issues, run), repo)
        append_summary('\n'.join(['### Flaky tests recorded', ''] + lines) if lines else '')
        print('\n'.join(lines) or 'No flaky tests.')
        return 0
    print('usage: flaky_tests.py check | summary JUNIT... | issues JUNIT|FOLDER...', file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
