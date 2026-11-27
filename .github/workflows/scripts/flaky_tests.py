"""Flaky tests in nextest's JUnit reports (GEL-389).

CI runs a failed test once more (nextest profile `ci`); one that then passes
is flaky and passes the run, so a flake costs no re-run. It must not pass
silently, though:

`summary JUNIT...` writes every flaky test to the job summary
($GITHUB_STEP_SUMMARY) and as a warning on the run.

`issues JUNIT|FOLDER...` (push runs, with an `issues: write` token) adds each flake
to its issue: the one .github/flaky-tests.toml names for the test, else the
open issue "Flaky test: <test>", which it opens the first time.

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
    """The flakes of every report that exists, and of every report in a
    folder (the downloaded artifacts); a job that failed before its tests
    ran wrote none."""
    found = []
    for path in map(Path, paths):
        for report in sorted(path.rglob('*.xml')) if path.is_dir() else [path] if path.is_file() else []:
            found.extend(flakes(report.read_text(encoding='utf-8')))
    return found


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
    """One `::warning` line per flaky test, shown on the run and the pull request."""
    lines = []
    for flake in found:
        first = (flake.message.splitlines() or [''])[0]
        lines.append('::warning title=Flaky test::' +
                     escape_data(f'{flake.test} failed, then passed on retry: {first}'))
    return lines


def plan(found, known, open_issues, run):
    """What `issues` does: ('comment', number, body) or ('open', title, body),
    one per flaky test. `open_issues` maps open issues' titles to numbers;
    `run` describes the run (url, branch, sha)."""
    actions = []
    for flake in dict((flake.test, flake) for flake in found).values():
        body = (f'`{flake.test}` failed and then passed on retry ({flake.attempts} attempts) '
                f'in [this run]({run["url"]}) on `{run["branch"]}` at {run["sha"]}.\n\n'
                f'First failure:\n\n{fence(flake.message)}\n')
        number = known.get(flake.test) or open_issues.get(title(flake.test))
        if number:
            actions.append(('comment', number, body))
        else:
            actions.append(('open', title(flake.test), body + (
                '\nCI retries a failed test once and passes it when the retry passes (GEL-389). '
                'Further flakes of this test are added here. Fix the test, or, while it is being '
                'fixed, list it in `.github/flaky-tests.toml` with the issue that tracks it.\n')))
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
    for action, target, body in actions:
        if action == 'comment':
            gh_api('--method', 'POST', f'repos/{repo}/issues/{target}/comments', '-f', f'body={body}')
            print(f'Added to #{target}')
        else:
            created = json.loads(gh_api('--method', 'POST', f'repos/{repo}/issues',
                                        '-f', f'title={target}', '-f', f'body={body}'))
            print(f'Opened #{created["number"]}: {target}')


def main(argv):
    command, paths = (argv[0], argv[1:]) if argv else ('', [])
    known = quarantine(QUARANTINE.read_text(encoding='utf-8'))
    if command == 'check':
        print(f'{len(known)} known flaky test(s), at most {CAP}.')
        return 0
    found = read_reports(paths)
    if command == 'summary':
        print('\n'.join(warnings(found)) or 'No flaky tests.')
        text = summary(found, known, os.environ.get('JOB_LABEL', 'this job'))
        if text and os.environ.get('GITHUB_STEP_SUMMARY'):
            with open(os.environ['GITHUB_STEP_SUMMARY'], 'a', encoding='utf-8') as out:
                out.write(text + '\n')
        return 0
    if command == 'issues':
        repo = os.environ['GITHUB_REPOSITORY']
        run = {'url': f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
               'branch': os.environ['GITHUB_REF_NAME'], 'sha': os.environ['GITHUB_SHA']}
        actions = plan(found, known, open_flaky_issues(repo) if found else {}, run)
        file_issues(actions, repo)
        print(f'{len(actions)} flaky test(s) recorded.')
        return 0
    print('usage: flaky_tests.py check | summary JUNIT... | issues JUNIT|FOLDER...', file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
